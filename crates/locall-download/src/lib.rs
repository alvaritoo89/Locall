//! # locall-download
//!
//! Descarga archivos de modelos **directamente al almacén** de Locall:
//!
//! - **Huella obligatoria:** se calcula *mientras* se descarga; si no coincide, el archivo se descarta.
//! - **Reanudable:** lo descargado se guarda en `tmp\sha256-<hex>.parcial`. Si la conexión se corta, si se pausa o si
//!   se cierra Locall, la próxima vez continúa desde ese byte (petición `Range`).
//! - **Detección de atascos:** si durante [`Opciones::espera_atasco`] no llega ni un byte, se corta y se reintenta.
//! - **Progreso fiable:** contado con los bytes que escribe el propio descargador (nunca con el tamaño que muestra el
//!   sistema de archivos: lección de la Fase 0).
//! - **Reintentos** con esperas crecientes; ante un `429` (demasiadas peticiones) espera lo que pide el servidor.
//! - **HTTPS** con el TLS de Windows y sus certificados (ADR 0010).
//!
//! El descargador es *bloqueante*: hay que llamarlo desde un hilo propio. Para pausar o cancelar, otro hilo pone a
//! `true` el `AtomicBool` que recibe [`Descargador::descargar`].

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use locall_store::{Almacen, ErrorAlmacen, Huella};
use sha2::{Digest, Sha256};
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

/// Tamaño de los trozos que se leen de la red (256 KiB).
const TROZO: usize = 256 * 1024;

/// Máxima espera que aceptamos de un `Retry-After` (un servidor no puede tenernos parados una hora).
const MAX_ESPERA_SERVIDOR: Duration = Duration::from_secs(120);

// ─── Tipos públicos ───────────────────────────────────────────────────────────

/// Qué descargar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pedido {
    /// URL fija del archivo, con la revisión exacta (p. ej. `https://huggingface.co/<repo>/resolve/<commit>/<archivo>`).
    pub url: String,
    /// Huella esperada (obligatoria).
    pub huella: Huella,
    /// Tamaño esperado en bytes.
    pub bytes: u64,
}

/// Progreso de una descarga.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progreso {
    pub descargados: u64,
    pub total: u64,
}

/// Ajustes del descargador.
#[derive(Debug, Clone)]
pub struct Opciones {
    /// Fallos seguidos (sin recibir datos entre ellos) antes de rendirse.
    pub fallos_seguidos_max: u32,
    /// Si no llega ningún byte durante este tiempo, la conexión se da por atascada.
    pub espera_atasco: Duration,
    /// Tiempo máximo para conectar.
    pub espera_conexion: Duration,
    /// Primera espera entre reintentos (se duplica en cada fallo seguido, hasta 30 s).
    pub espera_base: Duration,
    /// Cómo se identifica Locall ante el servidor.
    pub agente_usuario: String,
}

impl Default for Opciones {
    fn default() -> Self {
        Opciones {
            fallos_seguidos_max: 8,
            espera_atasco: Duration::from_secs(60),
            espera_conexion: Duration::from_secs(30),
            espera_base: Duration::from_secs(1),
            agente_usuario: format!("locall/{}", env!("CARGO_PKG_VERSION")),
        }
    }
}

/// Lo que puede salir mal.
#[derive(Debug)]
pub enum ErrorDescarga {
    /// Problema del almacén (espacio insuficiente, permisos…).
    Almacen(ErrorAlmacen),
    /// Error escribiendo el archivo parcial.
    Io(io::Error),
    /// El servidor respondió con un error que no se arregla reintentando (404, 403…).
    Http { estado: u16 },
    /// Se agotaron los reintentos; incluye el último error.
    IntentosAgotados { ultimo: String },
    /// Pausada o cancelada a petición: lo descargado se conserva para reanudar.
    Cancelada,
    /// El contenido no coincide con la huella esperada; se ha descartado.
    HuellaNoCoincide { esperada: Huella, obtenida: Huella },
    /// El servidor envió más datos de los esperados; se ha descartado.
    DemasiadosDatos { esperados: u64 },
}

impl fmt::Display for ErrorDescarga {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorDescarga::Almacen(e) => write!(f, "{e}"),
            ErrorDescarga::Io(e) => write!(f, "error al escribir la descarga: {e}"),
            ErrorDescarga::Http { estado } => {
                write!(f, "el servidor respondió con el error {estado}")
            }
            ErrorDescarga::IntentosAgotados { ultimo } => {
                write!(
                    f,
                    "no se pudo completar la descarga tras varios intentos ({ultimo})"
                )
            }
            ErrorDescarga::Cancelada => write!(f, "descarga pausada; se puede reanudar"),
            ErrorDescarga::HuellaNoCoincide { esperada, obtenida } => write!(
                f,
                "el archivo descargado no coincide con el original (esperada {esperada}, obtenida {obtenida}); \
                 se ha descartado"
            ),
            ErrorDescarga::DemasiadosDatos { esperados } => write!(
                f,
                "el servidor envió más de los {esperados} bytes esperados; se ha descartado"
            ),
        }
    }
}

impl std::error::Error for ErrorDescarga {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ErrorDescarga::Almacen(e) => Some(e),
            ErrorDescarga::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<ErrorAlmacen> for ErrorDescarga {
    fn from(e: ErrorAlmacen) -> Self {
        ErrorDescarga::Almacen(e)
    }
}

impl From<io::Error> for ErrorDescarga {
    fn from(e: io::Error) -> Self {
        ErrorDescarga::Io(e)
    }
}

// ─── El descargador ───────────────────────────────────────────────────────────

/// Resultado de un intento de descarga.
enum Intento {
    /// Se recibió todo lo esperado.
    Completo,
    /// Fallo recuperable: reintentar (tras esperar lo indicado, si lo hay).
    Reintentar {
        motivo: String,
        esperar: Option<Duration>,
    },
}

/// Descargador reutilizable (mantiene las conexiones abiertas entre descargas).
pub struct Descargador {
    agente: ureq::Agent,
    opciones: Opciones,
}

impl Descargador {
    pub fn nuevo(opciones: Opciones) -> Descargador {
        let tls = TlsConfig::builder()
            .provider(TlsProvider::NativeTls)
            .root_certs(RootCerts::PlatformVerifier) // los certificados de Windows
            .build();
        let config = ureq::Agent::config_builder()
            .tls_config(tls)
            .http_status_as_error(false) // los códigos de error los tratamos nosotros
            .timeout_connect(Some(opciones.espera_conexion))
            .timeout_per_read(Some(opciones.espera_atasco)) // detección de atascos
            .user_agent(opciones.agente_usuario.as_str())
            .build();
        Descargador {
            agente: ureq::Agent::new_with_config(config),
            opciones,
        }
    }

    /// Descarga `pedido` al almacén y devuelve su huella cuando está verificado y en `blobs\`.
    ///
    /// - Si el archivo ya está en el almacén, no descarga nada.
    /// - Si hay una descarga a medias, continúa desde donde se quedó.
    /// - `cancelar`: si otro hilo lo pone a `true`, se detiene y conserva lo descargado ([`ErrorDescarga::Cancelada`]).
    /// - `progreso`: se llama cada vez que llegan datos.
    pub fn descargar(
        &self,
        almacen: &Almacen,
        pedido: &Pedido,
        cancelar: &AtomicBool,
        mut progreso: impl FnMut(Progreso),
    ) -> Result<Huella, ErrorDescarga> {
        let total = pedido.bytes;
        if almacen.contiene(&pedido.huella) {
            log::info!(
                "descarga innecesaria: {} ya está en el almacén",
                pedido.huella
            );
            progreso(Progreso {
                descargados: total,
                total,
            });
            return Ok(pedido.huella.clone());
        }

        let ruta = almacen.ruta_parcial(&pedido.huella);
        let mut archivo = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true) // no "append": Windows no deja truncar un archivo abierto así
            .truncate(false)
            .open(&ruta)?;

        // Reanudación: volver a calcular la huella de lo que ya teníamos (el cursor queda al final, listo para seguir).
        let mut calculadora = Sha256::new();
        let mut descargados = huella_de_lo_existente(&mut archivo, &mut calculadora)?;
        if descargados > total {
            // Una parcial más grande de lo esperado no es de este archivo: empezar de cero.
            reiniciar(&mut archivo, &mut calculadora, &mut descargados)?;
        }
        almacen.comprobar_espacio(total - descargados)?;
        progreso(Progreso { descargados, total });
        if descargados > 0 {
            log::info!(
                "reanudando la descarga de {} desde el byte {descargados} de {total}",
                pedido.url
            );
        } else {
            log::info!("descargando {} ({total} bytes)", pedido.url);
        }
        let inicio = std::time::Instant::now();

        let mut fallos_seguidos = 0u32;
        while descargados < total {
            if cancelar.load(Ordering::Relaxed) {
                archivo.sync_all()?;
                log::info!("descarga pausada en el byte {descargados} de {total}");
                return Err(ErrorDescarga::Cancelada);
            }
            let antes = descargados;
            let resultado = self.intento(
                pedido,
                &mut archivo,
                &mut calculadora,
                &mut descargados,
                cancelar,
                &mut progreso,
            );
            let (motivo, esperar) = match resultado {
                Ok(Intento::Completo) => break,
                Ok(Intento::Reintentar { motivo, esperar }) => (motivo, esperar),
                Err(ErrorDescarga::DemasiadosDatos { esperados }) => {
                    drop(archivo);
                    let _ = std::fs::remove_file(&ruta);
                    return Err(ErrorDescarga::DemasiadosDatos { esperados });
                }
                Err(e) => return Err(e),
            };
            // Si en este intento llegaron datos, el contador de fallos seguidos vuelve a cero.
            fallos_seguidos = if descargados > antes {
                1
            } else {
                fallos_seguidos + 1
            };
            if fallos_seguidos > self.opciones.fallos_seguidos_max {
                archivo.sync_all()?;
                log::warn!("descarga abandonada tras {fallos_seguidos} fallos seguidos: {motivo}");
                return Err(ErrorDescarga::IntentosAgotados { ultimo: motivo });
            }
            let espera = esperar.unwrap_or_else(|| self.espera_creciente(fallos_seguidos));
            log::warn!(
                "descarga: reintento {fallos_seguidos} en {:.1} s ({motivo})",
                espera.as_secs_f64()
            );
            dormir_salvo_cancelacion(espera, cancelar);
        }

        // Verificación final.
        archivo.sync_all()?;
        drop(archivo);
        let obtenida = Huella::desde_sha256(&calculadora.finalize())?;
        if obtenida != pedido.huella {
            log::warn!(
                "descarga descartada: la huella no coincide (esperada {}, obtenida {obtenida})",
                pedido.huella
            );
            let _ = std::fs::remove_file(&ruta);
            return Err(ErrorDescarga::HuellaNoCoincide {
                esperada: pedido.huella.clone(),
                obtenida,
            });
        }
        almacen.confirmar_parcial(&pedido.huella)?;
        log::info!(
            "descarga verificada: {obtenida} ({total} bytes en {:.1} s)",
            inicio.elapsed().as_secs_f64()
        );
        Ok(obtenida)
    }

    /// Un intento: una petición HTTP y la lectura de su cuerpo.
    fn intento(
        &self,
        pedido: &Pedido,
        archivo: &mut File,
        calculadora: &mut Sha256,
        descargados: &mut u64,
        cancelar: &AtomicBool,
        progreso: &mut impl FnMut(Progreso),
    ) -> Result<Intento, ErrorDescarga> {
        let mut peticion = self.agente.get(&pedido.url);
        if *descargados > 0 {
            peticion = peticion.header("Range", &format!("bytes={descargados}-"));
        }
        let respuesta = match peticion.call() {
            Ok(r) => r,
            Err(e) => {
                return Ok(Intento::Reintentar {
                    motivo: format!("conexión: {e}"),
                    esperar: None,
                });
            }
        };

        let estado = respuesta.status().as_u16();
        match estado {
            206 => {
                // Comprobar que el servidor continúa exactamente donde le pedimos.
                let rango = respuesta
                    .headers()
                    .get("content-range")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                if !rango.starts_with(&format!("bytes {descargados}-")) {
                    reiniciar(archivo, calculadora, descargados)?;
                    return Ok(Intento::Reintentar {
                        motivo: format!("rango inesperado «{rango}»; se empieza de cero"),
                        esperar: Some(Duration::ZERO),
                    });
                }
            }
            200 => {
                // El servidor ignora la reanudación y envía el archivo entero: empezar de cero.
                if *descargados > 0 {
                    reiniciar(archivo, calculadora, descargados)?;
                    progreso(Progreso {
                        descargados: 0,
                        total: pedido.bytes,
                    });
                }
            }
            416 => {
                // "Rango no satisfacible": lo que tenemos no encaja con el archivo del servidor.
                reiniciar(archivo, calculadora, descargados)?;
                return Ok(Intento::Reintentar {
                    motivo: "rango no satisfacible; se empieza de cero".into(),
                    esperar: Some(Duration::ZERO),
                });
            }
            429 => {
                return Ok(Intento::Reintentar {
                    motivo: "el servidor pide esperar (429)".into(),
                    esperar: Some(espera_pedida(&respuesta)),
                });
            }
            500..=599 => {
                return Ok(Intento::Reintentar {
                    motivo: format!("error del servidor ({estado})"),
                    esperar: None,
                });
            }
            _ => return Err(ErrorDescarga::Http { estado }),
        }

        // Leer el cuerpo por trozos.
        let mut lector = respuesta.into_body().into_reader();
        let mut buffer = vec![0u8; TROZO];
        loop {
            if cancelar.load(Ordering::Relaxed) {
                archivo.sync_all()?;
                return Err(ErrorDescarga::Cancelada);
            }
            let leidos = match lector.read(&mut buffer) {
                Ok(0) => break, // fin del cuerpo
                Ok(n) => n,
                Err(e) => {
                    // Corte o atasco: lo recibido ya está guardado; se reanudará.
                    archivo.flush()?;
                    return Ok(Intento::Reintentar {
                        motivo: format!("lectura: {e}"),
                        esperar: None,
                    });
                }
            };
            if *descargados + leidos as u64 > pedido.bytes {
                return Err(ErrorDescarga::DemasiadosDatos {
                    esperados: pedido.bytes,
                });
            }
            archivo.write_all(&buffer[..leidos])?;
            calculadora.update(&buffer[..leidos]);
            *descargados += leidos as u64;
            progreso(Progreso {
                descargados: *descargados,
                total: pedido.bytes,
            });
        }
        archivo.flush()?;

        if *descargados == pedido.bytes {
            Ok(Intento::Completo)
        } else {
            Ok(Intento::Reintentar {
                motivo: format!(
                    "la conexión terminó antes de tiempo ({} de {} bytes)",
                    descargados, pedido.bytes
                ),
                esperar: None,
            })
        }
    }

    /// 1 s, 2 s, 4 s… hasta 30 s (con la espera base por defecto).
    fn espera_creciente(&self, fallos: u32) -> Duration {
        let factor = 1u32 << fallos.saturating_sub(1).min(5);
        (self.opciones.espera_base * factor).min(Duration::from_secs(30))
    }
}

// ─── Ayudantes ────────────────────────────────────────────────────────────────

/// Lee lo que ya hay en el archivo parcial y lo pasa por la calculadora de la huella. Devuelve los bytes leídos.
fn huella_de_lo_existente(archivo: &mut File, calculadora: &mut Sha256) -> io::Result<u64> {
    use std::io::{Seek, SeekFrom};
    archivo.seek(SeekFrom::Start(0))?;
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut total = 0u64;
    loop {
        let n = archivo.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        calculadora.update(&buffer[..n]);
        total += n as u64;
    }
    Ok(total)
}

/// Vacía el archivo parcial y la calculadora para empezar de cero.
fn reiniciar(
    archivo: &mut File,
    calculadora: &mut Sha256,
    descargados: &mut u64,
) -> io::Result<()> {
    use std::io::{Seek, SeekFrom};
    archivo.set_len(0)?;
    archivo.seek(SeekFrom::Start(0))?;
    *calculadora = Sha256::new();
    *descargados = 0;
    Ok(())
}

/// Lo que el servidor pide esperar en `Retry-After` (en segundos), con un tope. Si no lo dice, 10 s.
fn espera_pedida<B>(respuesta: &ureq::http::Response<B>) -> Duration {
    respuesta
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map_or(Duration::from_secs(10), Duration::from_secs)
        .min(MAX_ESPERA_SERVIDOR)
}

/// Duerme, pero se despierta enseguida si se pide cancelar.
fn dormir_salvo_cancelacion(espera: Duration, cancelar: &AtomicBool) {
    let paso = Duration::from_millis(100);
    let mut restante = espera;
    while !restante.is_zero() && !cancelar.load(Ordering::Relaxed) {
        let ahora = restante.min(paso);
        thread::sleep(ahora);
        restante -= ahora;
    }
}
