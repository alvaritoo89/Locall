//! # locall-download
//!
//! Downloads model files **straight into Locall's store**:
//!
//! - **Mandatory hash:** computed *while* downloading; if it does not match, the file is discarded.
//! - **Resumable:** what has been downloaded is kept in `tmp\sha256-<hex>.parcial`. If the connection drops, the
//!   download is paused or Locall is closed, the next run continues from that byte (`Range` request).
//! - **Stall detection:** if not a single byte arrives during [`Opciones::espera_atasco`], the connection is cut and
//!   retried.
//! - **Reliable progress:** counted with the bytes the downloader itself writes (never with the size the file system
//!   reports: a Phase 0 lesson).
//! - **Retries** with growing waits; on a `429` (too many requests) it waits as long as the server asks.
//! - **HTTPS** with Windows' TLS and its certificates (ADR 0010).
//!
//! The downloader is *blocking*: it must be called from its own thread. To pause or cancel, another thread sets to
//! `true` the `AtomicBool` that [`Descargador::descargar`] receives.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use locall_store::{Almacen, ErrorAlmacen, Huella};
use sha2::{Digest, Sha256};
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

/// Size of the chunks read from the network (256 KiB).
const TROZO: usize = 256 * 1024;

/// Longest `Retry-After` wait we accept (a server cannot keep us stopped for an hour).
const MAX_ESPERA_SERVIDOR: Duration = Duration::from_secs(120);

// ─── Public types ─────────────────────────────────────────────────────────────

/// What to download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pedido {
    /// Pinned URL of the file, with the exact revision (e.g. `https://huggingface.co/<repo>/resolve/<commit>/<file>`).
    pub url: String,
    /// Expected hash (mandatory).
    pub huella: Huella,
    /// Expected size in bytes.
    pub bytes: u64,
}

/// Progress of a download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progreso {
    pub descargados: u64,
    pub total: u64,
}

/// Downloader settings.
#[derive(Debug, Clone)]
pub struct Opciones {
    /// Consecutive failures (with no data received in between) before giving up.
    pub fallos_seguidos_max: u32,
    /// If no byte arrives during this time, the connection is considered stalled.
    pub espera_atasco: Duration,
    /// Maximum time to connect.
    pub espera_conexion: Duration,
    /// First wait between retries (doubles on each consecutive failure, up to 30 s).
    pub espera_base: Duration,
    /// How Locall identifies itself to the server.
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

/// What can go wrong.
#[derive(Debug)]
pub enum ErrorDescarga {
    /// Store problem (not enough space, permissions…).
    Almacen(ErrorAlmacen),
    /// Error writing the partial file.
    Io(io::Error),
    /// The server answered with an error that retrying does not fix (404, 403…).
    Http { estado: u16 },
    /// Retries ran out; includes the last error.
    IntentosAgotados { ultimo: String },
    /// Paused or cancelled on request: what was downloaded is kept for resuming.
    Cancelada,
    /// The content does not match the expected hash; it was discarded.
    HuellaNoCoincide { esperada: Huella, obtenida: Huella },
    /// The server sent more data than expected; it was discarded.
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

// ─── The downloader ───────────────────────────────────────────────────────────

/// Result of one download attempt.
enum Intento {
    /// Everything expected was received.
    Completo,
    /// Recoverable failure: retry (after waiting as indicated, if anything).
    Reintentar {
        motivo: String,
        esperar: Option<Duration>,
    },
}

/// Reusable downloader (keeps connections open between downloads).
pub struct Descargador {
    agente: ureq::Agent,
    opciones: Opciones,
}

impl Descargador {
    pub fn nuevo(opciones: Opciones) -> Descargador {
        let tls = TlsConfig::builder()
            .provider(TlsProvider::NativeTls)
            .root_certs(RootCerts::PlatformVerifier) // Windows' certificates
            .build();
        let config = ureq::Agent::config_builder()
            .tls_config(tls)
            .http_status_as_error(false) // we handle the error codes ourselves
            .timeout_connect(Some(opciones.espera_conexion))
            .timeout_per_read(Some(opciones.espera_atasco)) // stall detection
            .user_agent(opciones.agente_usuario.as_str())
            .build();
        Descargador {
            agente: ureq::Agent::new_with_config(config),
            opciones,
        }
    }

    /// Downloads `pedido` into the store and returns its hash once it is verified and in `blobs\`.
    ///
    /// - If the file is already in the store, nothing is downloaded.
    /// - If there is a half-finished download, it continues where it left off.
    /// - `cancelar`: if another thread sets it to `true`, it stops and keeps what was downloaded
    ///   ([`ErrorDescarga::Cancelada`]).
    /// - `progreso`: called every time data arrives.
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
            .write(true) // not "append": Windows does not allow truncating a file opened that way
            .truncate(false)
            .open(&ruta)?;

        // Resuming: recompute the hash of what we already had (the cursor ends at the end, ready to continue).
        let mut calculadora = Sha256::new();
        let mut descargados = huella_de_lo_existente(&mut archivo, &mut calculadora)?;
        if descargados > total {
            // A partial file bigger than expected does not belong to this file: start from scratch.
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
            // If data arrived in this attempt, the consecutive-failure counter goes back to zero.
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

        // Final check.
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

    /// One attempt: an HTTP request and reading its body.
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
                // Check that the server continues exactly where we asked.
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
                // The server ignores the resume and sends the whole file: start from scratch.
                if *descargados > 0 {
                    reiniciar(archivo, calculadora, descargados)?;
                    progreso(Progreso {
                        descargados: 0,
                        total: pedido.bytes,
                    });
                }
            }
            416 => {
                // "Range not satisfiable": what we have does not fit the server's file.
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

        // Read the body in chunks.
        let mut lector = respuesta.into_body().into_reader();
        let mut buffer = vec![0u8; TROZO];
        loop {
            if cancelar.load(Ordering::Relaxed) {
                archivo.sync_all()?;
                return Err(ErrorDescarga::Cancelada);
            }
            let leidos = match lector.read(&mut buffer) {
                Ok(0) => break, // end of the body
                Ok(n) => n,
                Err(e) => {
                    // Connection drop or stall: what was received is already saved; it will resume.
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

    /// 1 s, 2 s, 4 s… up to 30 s (with the default base wait).
    fn espera_creciente(&self, fallos: u32) -> Duration {
        let factor = 1u32 << fallos.saturating_sub(1).min(5);
        (self.opciones.espera_base * factor).min(Duration::from_secs(30))
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Reads what is already in the partial file and feeds it to the hash calculator. Returns the bytes read.
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

/// Empties the partial file and the calculator to start from scratch.
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

/// What the server asks us to wait in `Retry-After` (in seconds), capped. If it does not say, 10 s.
fn espera_pedida<B>(respuesta: &ureq::http::Response<B>) -> Duration {
    respuesta
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map_or(Duration::from_secs(10), Duration::from_secs)
        .min(MAX_ESPERA_SERVIDOR)
}

/// Sleeps, but wakes up right away if cancellation is requested.
fn dormir_salvo_cancelacion(espera: Duration, cancelar: &AtomicBool) {
    let paso = Duration::from_millis(100);
    let mut restante = espera;
    while !restante.is_zero() && !cancelar.load(Ordering::Relaxed) {
        let ahora = restante.min(paso);
        thread::sleep(ahora);
        restante -= ahora;
    }
}
