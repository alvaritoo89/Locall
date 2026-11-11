//! Downloader tests against a local **fake HTTP server** (127.0.0.1).
//!
//! This way we can trigger at will what happens on the Internet now and then: drops halfway, stalled connections,
//! "too many requests" (429), servers that ignore resuming, tampered content…
//! They do not use the network or download anything from outside.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use locall_download::{Descargador, ErrorDescarga, Opciones, Pedido};
use locall_store::{Almacen, huella_de_bytes};

// ─── Fake server ──────────────────────────────────────────────────────────────

/// A server that handles each connection with the function `manejar(request_number, requested_range, connection)`.
struct Servidor {
    url: String,
    /// The `Range` start of each request received (`None` = it asked for the whole file).
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
            // One thread per connection: a "stalled" connection must not block the next ones.
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

/// Reads the request header and returns the `Range` start (if any).
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

/// Writes a response header. `longitud` is the announced `Content-Length`.
fn cabecera(conexion: &mut TcpStream, estado: &str, longitud: usize, extra: &str) {
    let texto = format!(
        "HTTP/1.1 {estado}\r\nContent-Length: {longitud}\r\nConnection: close\r\n{extra}\r\n"
    );
    let _ = conexion.write_all(texto.as_bytes());
}

/// Answers like a correct server: 206 if a range was requested, 200 otherwise.
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

/// Announces the whole file but only sends up to `hasta` and closes (a dropped connection).
fn responder_cortado(conexion: &mut TcpStream, datos: &[u8], hasta: usize) {
    cabecera(conexion, "200 OK", datos.len(), "");
    let _ = conexion.write_all(&datos[..hasta]);
    let _ = conexion.flush();
}

// ─── Test helpers ─────────────────────────────────────────────────────────────

/// 300 KB of test data (more than one read chunk, so there are several reads).
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

/// Fast options for tests: stall after 0.5 s and 10 ms waits between retries.
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

// ─── Tests ────────────────────────────────────────────────────────────────────

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

    // If it is already in the store, nothing is requested from the server again.
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

    // The second request continues exactly where the first one was cut.
    assert_eq!(*srv.peticiones.lock().unwrap(), vec![None, Some(100_000)]);
}

#[test]
fn detecta_una_conexion_atascada() {
    let datos = datos_de_prueba();
    let d = Arc::clone(&datos);
    let srv = servidor(move |n, desde, c| {
        if n == 0 {
            // Sends a third and goes quiet without closing the connection.
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

    // It did not wait for the server to close (5 s): it detected the stall at 0.5 s and resumed.
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

    // It waited the second the server asked for (and not the normal 10 ms wait).
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
            // Ignores the Range and always sends the whole file with 200.
            responder_bien(c, &d, None);
        }
    });
    let carpeta = CarpetaDePrueba::nueva("sin-rango");
    let almacen = carpeta.almacen();

    let huella = descargar(&almacen, &pedido(&srv, &datos)).unwrap();

    // If the whole file had been appended after what was already downloaded, the hash would not match.
    assert_eq!(std::fs::read(almacen.ruta_blob(&huella)).unwrap(), *datos);
}

#[test]
fn descarta_un_contenido_adulterado() {
    let datos = datos_de_prueba();
    let mut adulterado = (*datos).clone();
    adulterado[1234] ^= 0xFF; // a single byte changed
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
    // 1 attempt + 3 retries (fallos_seguidos_max = 3).
    assert_eq!(srv.peticiones.lock().unwrap().len(), 4);
}

#[test]
fn se_pausa_y_se_reanuda() {
    let datos = datos_de_prueba();
    let d = Arc::clone(&datos);
    // A slow server: 16 KB every 20 ms.
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
                return; // the client went away (pause)
            }
            let _ = c.flush();
            thread::sleep(Duration::from_millis(20));
        }
    });
    let carpeta = CarpetaDePrueba::nueva("pausa");
    let almacen = carpeta.almacen();
    let pedido = pedido(&srv, &datos);
    let descargador = Descargador::nuevo(opciones_rapidas());

    // 1) It pauses after passing 64 KB.
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
    // Cleaning temporary files does not delete a paused download.
    almacen.limpiar_temporales().unwrap();
    assert!(parcial.exists());

    // 2) It resumes (as if it were another session: a new downloader) from what was saved.
    let huella = Descargador::nuevo(opciones_rapidas())
        .descargar(&almacen, &pedido, &AtomicBool::new(false), |_| {})
        .unwrap();
    assert!(almacen.contiene(&huella));
    let peticiones = srv.peticiones.lock().unwrap();
    assert_eq!(peticiones.last().copied().flatten(), Some(guardados));
}
