//! `locall-web [carpeta-de-salida]`: genera la web de Locall en la carpeta indicada (por defecto `target/web`).
//!
//! Lee `catalogo/catalogo.json` y su firma, y **se niega a generar nada si la firma no es buena** con la clave
//! oficial: así nunca se publica un catálogo sin firmar. Se ejecuta desde la raíz del repositorio (lo hace GitHub
//! Actions al publicar).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Archivo que deja el generador en la carpeta de salida: solo una carpeta con esta marca se vacía al regenerar.
const MARCA: &str = ".generada-por-locall-web";

fn main() -> ExitCode {
    let destino = std::env::args_os()
        .nth(1)
        .map_or_else(|| PathBuf::from("target").join("web"), PathBuf::from);
    match generar(&destino) {
        Ok(n) => {
            println!("Web generada en {} ({n} archivos).", destino.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn generar(destino: &Path) -> Result<usize, String> {
    let datos = std::fs::read("catalogo/catalogo.json").map_err(|e| {
        format!(
            "no se puede leer catalogo/catalogo.json ({e}); ejecútalo desde la raíz del repositorio"
        )
    })?;
    let firma = std::fs::read_to_string("catalogo/catalogo.json.minisig")
        .map_err(|e| format!("no se puede leer la firma del catálogo ({e})"))?;
    let catalogo = locall_catalogo::verificar(&datos, &firma, locall_catalogo::CLAVE_PUBLICA)
        .map_err(|e| format!("{e}. No se genera la web con un catálogo sin firma buena."))?;
    // Empezar de cero (que no quede la página de un modelo que ya no está), pero SOLO si la carpeta la creó este
    // programa (lleva su marca): nunca se vacía una carpeta cualquiera por un error al escribir la ruta.
    let vacia = std::fs::read_dir(destino).is_ok_and(|mut d| d.next().is_none());
    if destino.exists() && !vacia {
        if !destino.join(MARCA).is_file() {
            return Err(format!(
                "{} ya existe y no la creó locall-web; elige otra carpeta (o vacíala tú)",
                destino.display()
            ));
        }
        std::fs::remove_dir_all(destino)
            .map_err(|e| format!("no se puede vaciar {}: {e}", destino.display()))?;
    }
    let escritos = locall_web::generar(
        &catalogo,
        &datos,
        &firma,
        Path::new("web/estaticos"),
        destino,
    )
    .map_err(|e| format!("no se pudo escribir la web: {e}"))?;
    std::fs::write(
        destino.join(MARCA),
        "Carpeta generada por locall-web: se vacía al volver a generar.\n",
    )
    .map_err(|e| format!("no se pudo escribir la marca: {e}"))?;
    Ok(escritos.len())
}
