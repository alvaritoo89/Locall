//! Downloads a file into the store showing progress. Ctrl+C interrupts it (and the next run resumes).
//!
//! ```text
//! cargo run -p locall-download --example descargar -- <store_folder> <url> <sha256> <bytes>
//! ```

use std::io::Write;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use locall_download::{Descargador, Opciones, Pedido};
use locall_store::{Almacen, Huella};

fn main() {
    let argumentos: Vec<String> = std::env::args().skip(1).collect();
    let [carpeta, url, sha256, bytes] = argumentos.as_slice() else {
        eprintln!("uso: descargar <carpeta_almacen> <url> <sha256> <bytes>");
        std::process::exit(2);
    };

    let almacen = Almacen::abrir(carpeta).unwrap_or_else(|e| salir(&e));
    let pedido = Pedido {
        url: url.clone(),
        huella: Huella::desde_hex(sha256).unwrap_or_else(|e| salir(&e)),
        bytes: bytes.parse().unwrap_or_else(|e| salir(&e)),
    };

    let inicio = Instant::now();
    let resultado = Descargador::nuevo(Opciones::default()).descargar(
        &almacen,
        &pedido,
        &AtomicBool::new(false),
        |p| {
            let porcentaje = p.descargados as f64 * 100.0 / p.total.max(1) as f64;
            print!(
                "\r{porcentaje:5.1} %  ({:.1} de {:.1} MB)",
                p.descargados as f64 / 1e6,
                p.total as f64 / 1e6
            );
            let _ = std::io::stdout().flush();
        },
    );
    println!();
    match resultado {
        Ok(huella) => println!(
            "listo en {:.1} s → {}",
            inicio.elapsed().as_secs_f64(),
            almacen.ruta_blob(&huella).display()
        ),
        Err(e) => salir(&e),
    }
}

fn salir(error: &dyn std::fmt::Display) -> ! {
    eprintln!("error: {error}");
    std::process::exit(1);
}
