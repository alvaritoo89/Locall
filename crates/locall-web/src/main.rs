//! `locall-web [output-folder]`: generates the Locall website in the given folder (`target/web` by default).
//!
//! Reads `catalogo/catalogo.json` and its signature, and **refuses to generate anything if the signature is not valid**
//! with the official key: so an unsigned catalog is never published. Run it from the repository root (GitHub Actions
//! does this when publishing).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// File the generator leaves in the output folder: only a folder carrying this marker is emptied when regenerating.
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
    // Start from scratch (so no page is left for a model that is gone), but ONLY if the folder was created by this
    // program (it carries its marker): an arbitrary folder is never emptied because of a typo in the path.
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
