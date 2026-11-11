//! **Real** tests against Hugging Face (HTTPS with Windows' TLS, redirects to its CDN, `Range`…).
//!
//! They use the internet, so they are marked `#[ignore]`: a normal `cargo test` does not run them. To run them:
//!
//! ```text
//! cargo test -p locall-download --test hugging_face -- --ignored
//! ```
//!
//! They download tiny models (1.2 MB and 19 MB) from `ggml-org/tiny-llamas` (MIT license), pinned by revision and
//! hash, into a temporary folder that is deleted at the end.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use locall_download::{Descargador, ErrorDescarga, Opciones, Pedido};
use locall_store::{Almacen, Huella};

const REVISION: &str = "99dd1a73db5a37100bd4ae633f4cfce6560e1567";

fn pedido(archivo: &str, sha256: &str, bytes: u64) -> Pedido {
    Pedido {
        url: format!("https://huggingface.co/ggml-org/tiny-llamas/resolve/{REVISION}/{archivo}"),
        huella: Huella::desde_hex(sha256).unwrap(),
        bytes,
    }
}

struct CarpetaDePrueba(PathBuf);

impl CarpetaDePrueba {
    fn nueva(nombre: &str) -> CarpetaDePrueba {
        let ruta = std::env::temp_dir().join(format!("locall-hf-{nombre}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ruta);
        CarpetaDePrueba(ruta)
    }
}

impl Drop for CarpetaDePrueba {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
#[ignore = "usa internet (Hugging Face)"]
fn descarga_un_modelo_real() {
    let carpeta = CarpetaDePrueba::nueva("260k");
    let almacen = Almacen::abrir(carpeta.0.clone()).unwrap();
    let pedido = pedido(
        "stories260K.gguf",
        "047bf46455a544931cff6fef14d7910154c56afbc23ab1c5e56a72e69912c04b",
        1_185_376,
    );

    let huella = Descargador::nuevo(Opciones::default())
        .descargar(&almacen, &pedido, &AtomicBool::new(false), |_| {})
        .unwrap();

    assert!(almacen.contiene(&huella));
    assert_eq!(
        std::fs::metadata(almacen.ruta_blob(&huella)).unwrap().len(),
        1_185_376
    );
}

#[test]
#[ignore = "usa internet (Hugging Face)"]
fn pausa_y_reanuda_un_modelo_real() {
    let carpeta = CarpetaDePrueba::nueva("15m");
    let almacen = Almacen::abrir(carpeta.0.clone()).unwrap();
    let pedido = pedido(
        "stories15M-q4_0.gguf",
        "6151b1929d7f5aa3385d9ddef3393e55587c0a55de661562322bc51dfda93a04",
        19_077_344,
    );

    // 1) Pause after passing 5 MB.
    let cancelar = AtomicBool::new(false);
    let error = Descargador::nuevo(Opciones::default())
        .descargar(&almacen, &pedido, &cancelar, |p| {
            if p.descargados >= 5_000_000 {
                cancelar.store(true, Ordering::Relaxed);
            }
        })
        .unwrap_err();
    assert!(matches!(error, ErrorDescarga::Cancelada), "{error:?}");
    let guardados = std::fs::metadata(almacen.ruta_parcial(&pedido.huella))
        .unwrap()
        .len();
    assert!((5_000_000..19_077_344).contains(&guardados), "{guardados}");

    // 2) Resume: the first progress report must start from what was already saved (not from zero).
    let mut primero = None;
    let huella = Descargador::nuevo(Opciones::default())
        .descargar(&almacen, &pedido, &AtomicBool::new(false), |p| {
            primero.get_or_insert(p.descargados);
        })
        .unwrap();
    assert_eq!(primero, Some(guardados));
    assert!(almacen.contiene(&huella));
}
