//! Pruebas del descargador contra un **servidor HTTP falso** local (127.0.0.1).
//!
//! Así podemos provocar a voluntad lo que en Internet pasa de vez en cuando: cortes a mitad, conexiones atascadas,
//! "demasiadas peticiones" (429), servidores que ignoran la reanudación, contenido adulterado…
//! No usan la red ni descargan nada de fuera.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use locall_download::{Descargador, ErrorDescarga, Opciones, Pedido};
use locall_store::{Almacen, huella_de_bytes};

// ─── Servidor falso ───────────────────────────────────────────────────────────

/// Un servidor que atiende cada conexión con la función `manejar(número_de_petición, rango_pedido, conexión)`.
struct Servidor {
    url: String,
    /// El inicio del `Range` de cada petición recibida (`None` = pidió el archivo entero).
    peticiones: Arc<Mutex<Vec<Option<u64>>>>,
}

fn servidor<F>(manejar: F) -> Servidor
where
    F: Fn(usize, Option<u64>, &mut TcpStream) + Send + Sync + 'static,
{
    let escucha = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/modelo.gguf", escucha.local_addr().unwrap());
    let peticiones = Arc::new(Mutex::new(Vec::new()));
    let manejar = Arc::new(manejar);
    let registro = Arc::clone(&peticiones);
    thread::spawn(move || {
        for conexion in escucha.incoming() {
            let Ok(mut conexion) = conexion else { continue };
            let manejar = Arc::clone(&manejar);
            let registro = Arc::clone(&registro);
            // Un hilo por conexión: una conexión "atascada" no debe bloquear las siguientes.
            thread::spawn(move || {
                let desde = leer_peticion(&conexion);
                let numero = {
                    let mut lista = registro.lock().unwrap();
                    lista.push(desde);
                    lista.len() - 1
                };
                manejar(numero, desde, &mut conexion);
            });
        }
    });
    Servidor { url, peticiones }
}

/// Lee la cabecera de la petición y devuelve el inicio del `Range` (si lo hay).
fn leer_peticion(conexion: &TcpStream) -> Option<u64> {
    let mut lector = BufReader::new(conexion);
    let mut desde = None;
    loop {
        let mut linea = String::new();
        if lector.read_line(&mut linea).unwrap_or(0) == 0 || linea == "\r\n" {
            break;
        }
        let minusculas = linea.to_ascii_lowercase();
        if let Some(valor) = minusculas.strip_prefix("range: bytes=") {
            desde = valor.trim().trim_end_matches('-').parse().ok();
        }
    }
    desde
}

/// Escribe una cabecera de respuesta. `longitud` es el `Content-Length` que se anuncia.
fn cabecera(conexion: &mut TcpStream, estado: &str, longitud: usize, extra: &str) {
    let texto = format!(
        "HTTP/1.1 {estado}\r\nContent-Length: {longitud}\r\nConnection: close\r\n{extra}\r\n"
    );
    let _ = conexion.write_all(texto.as_bytes());
}

/// Responde como un servidor correcto: 206 si se pidió un rango, 200 si no.
fn responder_bien(conexion: &mut TcpStream, datos: &[u8], desde: Option<u64>) {
    match desde {
        Some(d) => {
            let d = d as usize;
            let rango = format!(
                "Content-Range: bytes {d}-{}/{}\r\n",
                datos.len() - 1,
                datos.len()
            );
            cabecera(conexion, "206 Partial Content", datos.len() - d, &rango);
            let _ = conexion.write_all(&datos[d..]);
        }
        None => {
            cabecera(conexion, "200 OK", datos.len(), "");
            let _ = conexion.write_all(datos);
        }
    }
}

/// Anuncia el archivo entero pero solo envía hasta `hasta` y cierra (un corte de conexión).
fn responder_cortado(conexion: &mut TcpStream, datos: &[u8], hasta: usize) {
    cabecera(conexion, "200 OK", datos.len(), "");
    let _ = conexion.write_all(&datos[..hasta]);
    let _ = conexion.flush();
}

// ─── Ayudantes de las pruebas ────────────────────────────────────────────────

/// 300 KB de datos de prueba (más que un trozo de lectura, para que haya varias lecturas).
fn datos_de_prueba() -> Arc<Vec<u8>> {
    Arc::new((0..300_000u32).map(|i| (i * 31 % 251) as u8).collect())
}

struct CarpetaDePrueba(PathBuf);

impl CarpetaDePrueba {
    fn nueva(nombre: &str) -> CarpetaDePrueba {
        let ruta =
            std::env::temp_dir().join(format!("locall-descarga-{nombre}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ruta);
        CarpetaDePrueba(ruta)
    }

    fn almacen(&self) -> Almacen {
        Almacen::abrir(self.0.clone()).unwrap()
    }
}

impl Drop for CarpetaDePrueba {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Opciones rápidas para pruebas: atasco a los 0,5 s y esperas de 10 ms entre reintentos.
fn opciones_rapidas() -> Opciones {
    Opciones {
        fallos_seguidos_max: 3,
        espera_atasco: Duration::from_millis(500),
        espera_conexion: Duration::from_secs(2),
        espera_base: Duration::from_millis(10),
        ..Opciones::default()
    }
}

fn pedido(servidor: &Servidor, datos: &[u8]) -> Pedido {
    Pedido {
        url: servidor.url.clone(),
        huella: huella_de_bytes(datos),
        bytes: datos.len() as u64,
    }
}

fn descargar(almacen: &Almacen, pedido: &Pedido) -> Result<locall_store::Huella, ErrorDescarga> {
    Descargador::nuevo(opciones_rapidas()).descargar(
        almacen,
        pedido,
        &AtomicBool::new(false),
        |_| {},
    )
}

// ─── Pruebas ──────────────────────────────────────────────────────────────────

#[test]
fn descarga_normal() {
    let datos = datos_de_prueba();
    let d = Arc::clone(&datos);
    let srv = servidor(move |_, desde, c| responder_bien(c, &d, desde));
    let carpeta = CarpetaDePrueba::nueva("normal");
    let almacen = carpeta.almacen();
    let pedido = pedido(&srv, &datos);

    let mut ultimo = None;
    let huella = Descargador::nuevo(opciones_rapidas())
        .descargar(&almacen, &pedido, &AtomicBool::new(false), |p| {
            ultimo = Some(p)
        })
        .unwrap();

    assert_eq!(huella, pedido.huella);
    assert!(almacen.contiene(&huella));
    assert!(
        !almacen.ruta_parcial(&huella).exists(),
        "la parcial desaparece al confirmar"
    );
    assert_eq!(std::fs::read(almacen.ruta_blob(&huella)).unwrap(), *datos);
    let ultimo = ultimo.unwrap();
    assert_eq!(ultimo.descargados, ultimo.total);

    // Si ya está en el almacén, no se vuelve a pedir nada al servidor.
    descargar(&almacen, &pedido).unwrap();
    assert_eq!(srv.peticiones.lock().unwrap().len(), 1);
}

#[test]
fn se_reanuda_tras_un_corte() {
    let datos = datos_de_prueba();
    let d = Arc::clone(&datos);
    let srv = servidor(move |n, desde, c| {
        if n == 0 {
            responder_cortado(c, &d, 100_000);
        } else {
            responder_bien(c, &d, desde);
        }
    });
    let carpeta = CarpetaDePrueba::nueva("corte");
    let almacen = carpeta.almacen();

    descargar(&almacen, &pedido(&srv, &datos)).unwrap();

    // La segunda petición continúa exactamente donde se cortó la primera.
    assert_eq!(*srv.peticiones.lock().unwrap(), vec![None, Some(100_000)]);
}

#[test]
fn detecta_una_conexion_atascada() {
    let datos = datos_de_prueba();
    let d = Arc::clone(&datos);
    let srv = servidor(move |n, desde, c| {
        if n == 0 {
            // Envía un tercio y se queda callado sin cerrar la conexión.
            cabecera(c, "200 OK", d.len(), "");
            let _ = c.write_all(&d[..100_000]);
            let _ = c.flush();
            thread::sleep(Duration::from_secs(5));
        } else {
            responder_bien(c, &d, desde);
        }
    });
    let carpeta = CarpetaDePrueba::nueva("atasco");
    let almacen = carpeta.almacen();

    let inicio = Instant::now();
    descargar(&almacen, &pedido(&srv, &datos)).unwrap();

    // No esperó a que el servidor cerrase (5 s): detectó el atasco a los 0,5 s y reanudó.
    assert!(
        inicio.elapsed() < Duration::from_secs(4),
        "tardó {:?}",
        inicio.elapsed()
    );
    assert_eq!(*srv.peticiones.lock().unwrap(), vec![None, Some(100_000)]);
}

#[test]
fn respeta_el_retry_after_de_un_429() {
    let datos = datos_de_prueba();
    let d = Arc::clone(&datos);
    let srv = servidor(move |n, desde, c| {
        if n == 0 {
            cabecera(c, "429 Too Many Requests", 0, "Retry-After: 1\r\n");
        } else {
            responder_bien(c, &d, desde);
        }
    });
    let carpeta = CarpetaDePrueba::nueva("429");
    let almacen = carpeta.almacen();

    let inicio = Instant::now();
    descargar(&almacen, &pedido(&srv, &datos)).unwrap();

    // Esperó el segundo que pidió el servidor (y no los 10 ms de la espera normal).
    assert!(
        inicio.elapsed() >= Duration::from_millis(950),
        "tardó {:?}",
        inicio.elapsed()
    );
    assert_eq!(srv.peticiones.lock().unwrap().len(), 2);
}

#[test]
fn empieza_de_cero_si_el_servidor_ignora_la_reanudacion() {
    let datos = datos_de_prueba();
    let d = Arc::clone(&datos);
    let srv = servidor(move |n, _desde, c| {
        if n == 0 {
            responder_cortado(c, &d, 100_000);
        } else {
            // Ignora el Range y envía siempre el archivo entero con 200.
            responder_bien(c, &d, None);
        }
    });
    let carpeta = CarpetaDePrueba::nueva("sin-rango");
    let almacen = carpeta.almacen();

    let huella = descargar(&almacen, &pedido(&srv, &datos)).unwrap();

    // Si se hubiera añadido el archivo entero tras lo ya descargado, la huella no coincidiría.
    assert_eq!(std::fs::read(almacen.ruta_blob(&huella)).unwrap(), *datos);
}

#[test]
fn descarta_un_contenido_adulterado() {
    let datos = datos_de_prueba();
    let mut adulterado = (*datos).clone();
    adulterado[1234] ^= 0xFF; // un solo byte cambiado
    let srv = servidor(move |_, desde, c| responder_bien(c, &adulterado, desde));
    let carpeta = CarpetaDePrueba::nueva("adulterado");
    let almacen = carpeta.almacen();
    let pedido = pedido(&srv, &datos);

    let error = descargar(&almacen, &pedido).unwrap_err();

    assert!(
        matches!(error, ErrorDescarga::HuellaNoCoincide { .. }),
        "{error:?}"
    );
    assert!(!almacen.contiene(&pedido.huella));
    assert!(
        !almacen.ruta_parcial(&pedido.huella).exists(),
        "lo adulterado se borra"
    );
}

#[test]
fn descarta_si_el_servidor_envia_de_mas() {
    let datos = datos_de_prueba();
    let mut de_mas = (*datos).clone();
    de_mas.extend_from_slice(b"sobra");
    let srv = servidor(move |_, desde, c| responder_bien(c, &de_mas, desde));
    let carpeta = CarpetaDePrueba::nueva("de-mas");
    let almacen = carpeta.almacen();
    let pedido = pedido(&srv, &datos);

    let error = descargar(&almacen, &pedido).unwrap_err();

    assert!(
        matches!(error, ErrorDescarga::DemasiadosDatos { .. }),
        "{error:?}"
    );
    assert!(!almacen.ruta_parcial(&pedido.huella).exists());
}

#[test]
fn un_404_no_se_reintenta() {
    let srv = servidor(|_, _, c| cabecera(c, "404 Not Found", 0, ""));
    let carpeta = CarpetaDePrueba::nueva("404");
    let almacen = carpeta.almacen();

    let error = descargar(&almacen, &pedido(&srv, b"lo que sea")).unwrap_err();

    assert!(
        matches!(error, ErrorDescarga::Http { estado: 404 }),
        "{error:?}"
    );
    assert_eq!(srv.peticiones.lock().unwrap().len(), 1);
}

#[test]
fn se_rinde_tras_varios_errores_seguidos_del_servidor() {
    let srv = servidor(|_, _, c| cabecera(c, "503 Service Unavailable", 0, ""));
    let carpeta = CarpetaDePrueba::nueva("503");
    let almacen = carpeta.almacen();

    let error = descargar(&almacen, &pedido(&srv, b"lo que sea")).unwrap_err();

    assert!(
        matches!(error, ErrorDescarga::IntentosAgotados { .. }),
        "{error:?}"
    );
    // 1 intento + 3 reintentos (fallos_seguidos_max = 3).
    assert_eq!(srv.peticiones.lock().unwrap().len(), 4);
}

#[test]
fn se_pausa_y_se_reanuda() {
    let datos = datos_de_prueba();
    let d = Arc::clone(&datos);
    // Un servidor lento: 16 KB cada 20 ms.
    let srv = servidor(move |_, desde, c| {
        let inicio = desde.unwrap_or(0) as usize;
        match desde {
            Some(_) => {
                let rango = format!(
                    "Content-Range: bytes {inicio}-{}/{}\r\n",
                    d.len() - 1,
                    d.len()
                );
                cabecera(c, "206 Partial Content", d.len() - inicio, &rango);
            }
            None => cabecera(c, "200 OK", d.len(), ""),
        }
        for trozo in d[inicio..].chunks(16 * 1024) {
            if c.write_all(trozo).is_err() {
                return; // el cliente se fue (pausa)
            }
            let _ = c.flush();
            thread::sleep(Duration::from_millis(20));
        }
    });
    let carpeta = CarpetaDePrueba::nueva("pausa");
    let almacen = carpeta.almacen();
    let pedido = pedido(&srv, &datos);
    let descargador = Descargador::nuevo(opciones_rapidas());

    // 1) Se pausa al pasar de 64 KB.
    let cancelar = AtomicBool::new(false);
    let error = descargador
        .descargar(&almacen, &pedido, &cancelar, |p| {
            if p.descargados >= 64 * 1024 {
                cancelar.store(true, Ordering::Relaxed);
            }
        })
        .unwrap_err();
    assert!(matches!(error, ErrorDescarga::Cancelada), "{error:?}");
    let parcial = almacen.ruta_parcial(&pedido.huella);
    let guardados = std::fs::metadata(&parcial).unwrap().len();
    assert!(
        guardados >= 64 * 1024 && guardados < datos.len() as u64,
        "{guardados}"
    );
    // Limpiar temporales no borra una descarga pausada.
    almacen.limpiar_temporales().unwrap();
    assert!(parcial.exists());

    // 2) Se reanuda (como si fuese otra sesión: un descargador nuevo) desde lo guardado.
    let huella = Descargador::nuevo(opciones_rapidas())
        .descargar(&almacen, &pedido, &AtomicBool::new(false), |_| {})
        .unwrap();
    assert!(almacen.contiene(&huella));
    let peticiones = srv.peticiones.lock().unwrap();
    assert_eq!(peticiones.last().copied().flatten(), Some(guardados));
}
