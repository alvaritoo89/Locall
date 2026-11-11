//! `locall pull`, `locall import` and `locall rm`.

use std::io::{BufRead, BufReader, IsTerminal, Write};
use std::path::Path;

use serde_json::{Value, json};

use crate::cliente::{Cliente, ErrorCli};
use crate::formato;
use crate::progreso::Barra;

/// `locall pull <repo>`: lists the repository's GGUFs for the user to choose.
pub fn listar_repo(
    cliente: &Cliente,
    repo: &str,
    revision: Option<&str>,
) -> Result<String, ErrorCli> {
    let (dueno, nombre) = repo.split_once('/').ok_or_else(|| {
        ErrorCli::Uso(format!(
            "«{repo}» no es un repositorio: se escribe propietario/nombre"
        ))
    })?;
    let mut ruta = format!("/api/hub/{}/{}", codificar(dueno), codificar(nombre));
    if let Some(r) = revision {
        ruta.push_str(&format!("?revision={}", codificar(r)));
    }
    let info = cliente.get(&ruta)?;
    let archivos = info["files"].as_array().cloned().unwrap_or_default();
    if archivos.is_empty() {
        return Ok(format!("«{repo}» no tiene modelos GGUF.\n"));
    }
    let filas: Vec<Vec<String>> = archivos
        .iter()
        .map(|a| {
            vec![
                a["path"].as_str().unwrap_or("").to_owned(),
                formato::tamano(a["size_bytes"].as_u64().unwrap_or(0)),
            ]
        })
        .collect();
    let mut texto = format!(
        "{repo} · commit {} · licencia {}\n\n",
        &info["commit"].as_str().unwrap_or("?")
            [..info["commit"].as_str().map_or(1, |c| c.len().min(12))],
        info["license"].as_str().unwrap_or("sin indicar"),
    );
    texto.push_str(&formato::tabla(&["ARCHIVO", "TAMAÑO"], &filas));
    if info["gated"].as_bool() == Some(true) {
        texto.push_str(
            "\nEste repositorio exige aceptar condiciones con una cuenta de Hugging Face: Locall todavía no puede \
             descargarlo.\n",
        );
    } else {
        let primero = filas[0][0].rsplit('/').next().unwrap_or("");
        texto.push_str(&format!(
            "\nPara instalar uno: locall pull {repo} {primero}\n"
        ));
    }
    Ok(texto)
}

/// `locall pull <repo> <file>`: installs with a progress bar.
pub fn instalar(
    cliente: &Cliente,
    repo: &str,
    archivo: &str,
    revision: Option<&str>,
    id: Option<&str>,
) -> Result<String, ErrorCli> {
    let mut cuerpo = json!({"repo": repo, "file": archivo});
    if let Some(r) = revision {
        cuerpo["revision"] = json!(r);
    }
    if let Some(i) = id {
        cuerpo["id"] = json!(i);
    }
    let _ = writeln!(std::io::stderr(), "Comprobando {archivo} en {repo}…");
    instalar_con(cliente, &cuerpo)
}

/// `locall pull <name>`: a catalog model.
pub fn instalar_del_catalogo(cliente: &Cliente, nombre: &str) -> Result<String, ErrorCli> {
    let _ = writeln!(
        std::io::stderr(),
        "Comprobando «{nombre}» contra el catálogo firmado…"
    );
    instalar_con(cliente, &json!({"name": nombre}))
}

/// `locall catalog`: the catalog models. First it checks whether there is a new one (at most once a day); if it
/// cannot (no internet), it shows the one it has.
pub fn catalogo(cliente: &Cliente) -> Result<String, ErrorCli> {
    let (catalogo, aviso) = match cliente.post("/api/catalog/update", &json!({})) {
        Ok(r) => (r["catalog"].clone(), None),
        Err(ErrorCli::Api { mensaje, .. }) => (
            cliente.get("/api/catalog")?,
            Some(format!(
                "No se pudo buscar un catálogo más nuevo: {mensaje}.\n"
            )),
        ),
        Err(e) => return Err(e),
    };
    let filas: Vec<Vec<String>> = catalogo["models"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|m| {
            vec![
                m["name"].as_str().unwrap_or("?").to_owned(),
                m["title"].as_str().unwrap_or("").to_owned(),
                formato::tamano(m["size_bytes"].as_u64().unwrap_or(0)),
                m["license"].as_str().unwrap_or("—").to_owned(),
                match m["fit_estimate"].as_str() {
                    Some("comfortable") => "de sobra",
                    Some("fits") => "sí",
                    Some("tight") => "justo",
                    Some("too_big") => "no",
                    _ => "—",
                }
                .to_owned(),
                if m["installed"].as_bool() == Some(true) {
                    "instalado".to_owned()
                } else {
                    String::new()
                },
            ]
        })
        .collect();
    let mut texto = aviso.unwrap_or_default();
    texto.push_str(&formato::tabla(
        &[
            "NOMBRE",
            "MODELO",
            "TAMAÑO",
            "LICENCIA",
            "¿CABE? (APROX.)",
            "",
        ],
        &filas,
    ));
    texto.push_str(
        "\nTodos son compatibles: el motor de Locall reconoce su formato. No se han probado uno a uno.\n\
         Para instalar uno: locall pull <nombre>\n",
    );
    Ok(texto)
}

/// Requests the installation and shows the progress bar until it finishes.
fn instalar_con(cliente: &Cliente, cuerpo: &Value) -> Result<String, ErrorCli> {
    let respuesta = cliente.post_sin_leer("/api/models/pull", cuerpo)?;
    let mut barra = Barra::nueva();
    for linea in BufReader::new(respuesta.into_body().into_reader()).lines() {
        let linea = linea.map_err(|e| ErrorCli::Red(format!("la descarga se cortó: {e}")))?;
        let Ok(evento) = serde_json::from_str::<Value>(&linea) else {
            continue;
        };
        match evento["type"].as_str() {
            Some("progress") => barra.actualizar(
                evento["downloaded_bytes"].as_u64().unwrap_or(0),
                evento["total_bytes"].as_u64().unwrap_or(0),
                evento["part"].as_u64().unwrap_or(1),
                evento["parts"].as_u64().unwrap_or(1),
            ),
            Some("done") => {
                barra.terminar();
                let m = &evento["model"];
                let id = m["id"].as_str().unwrap_or("?");
                return Ok(format!(
                    "Instalado «{id}» ({}, verificado con sha256).\nPruébalo: locall run {id}\n",
                    formato::tamano(m["size_bytes"].as_u64().unwrap_or(0)),
                ));
            }
            Some("error") => {
                barra.terminar();
                return Err(ErrorCli::Api {
                    estado: 0,
                    codigo: evento["error"]["code"].as_str().unwrap_or("").to_owned(),
                    mensaje: evento["error"]["message"]
                        .as_str()
                        .unwrap_or("error")
                        .to_owned(),
                });
            }
            _ => {}
        }
    }
    barra.terminar();
    Err(ErrorCli::Red(
        "la descarga se cortó; vuelve a ejecutar la misma orden para continuar donde se quedó"
            .into(),
    ))
}

/// `locall import <path>`
pub fn importar(cliente: &Cliente, ruta: &str, id: Option<&str>) -> Result<String, ErrorCli> {
    // The API asks for a full path: the command's one may be relative to the current folder.
    let completa = std::path::absolute(Path::new(ruta))
        .map_err(|e| ErrorCli::Uso(format!("ruta no válida «{ruta}»: {e}")))?;
    let mut cuerpo = json!({"path": completa.to_string_lossy()});
    if let Some(i) = id {
        cuerpo["id"] = json!(i);
    }
    let _ = writeln!(std::io::stderr(), "Comprobando {}…", completa.display());
    let m = cliente.post("/api/models/import", &cuerpo)?;
    let id = m["id"].as_str().unwrap_or("?");
    Ok(format!(
        "Importado «{id}» ({}). Si estaba en el mismo disco que Locall, no se ha copiado: no ocupa el doble.\n\
         Pruébalo: locall run {id}\n",
        formato::tamano(m["size_bytes"].as_u64().unwrap_or(0)),
    ))
}

/// `locall rm <id>`: asks for confirmation unless `--yes` is given.
pub fn eliminar(cliente: &Cliente, id: &str, sin_preguntar: bool) -> Result<String, ErrorCli> {
    // First, check that it exists (and how much space it takes) so the question can include the data.
    let m = cliente.get(&format!("/api/models/{}", codificar(id)))?;
    if !sin_preguntar {
        if !std::io::stdin().is_terminal() {
            return Err(ErrorCli::Uso(format!(
                "para eliminar «{id}» sin preguntar, añade --yes"
            )));
        }
        let mut error = std::io::stderr();
        let _ = write!(
            error,
            "¿Eliminar «{id}» ({})? Se borrarán sus archivos si ningún otro modelo los usa. [s/N] ",
            formato::tamano(m["size_bytes"].as_u64().unwrap_or(0))
        );
        let _ = error.flush();
        let mut respuesta = String::new();
        let _ = std::io::stdin().read_line(&mut respuesta);
        if !es_si(&respuesta) {
            return Ok("No se ha eliminado nada.\n".into());
        }
    }
    let r = cliente.delete(&format!("/api/models/{}", codificar(id)))?;
    Ok(format!(
        "Eliminado «{id}»: {} liberados.\n",
        formato::tamano(r["freed_bytes"].as_u64().unwrap_or(0))
    ))
}

/// Did they answer yes? (s, si, sí, y, yes; upper or lower case).
pub fn es_si(respuesta: &str) -> bool {
    matches!(
        respuesta.trim().to_lowercase().as_str(),
        "s" | "si" | "sí" | "y" | "yes"
    )
}

/// Just enough to put a text into an API path (valid identifiers need nothing).
pub fn codificar(texto: &str) -> String {
    let mut salida = String::new();
    for byte in texto.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
            salida.push(byte as char);
        } else {
            salida.push_str(&format!("%{byte:02X}"));
        }
    }
    salida
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn respuestas_afirmativas() {
        for si in ["s", "S", "si", "Sí", "y", "YES", " s\r\n"] {
            assert!(es_si(si), "{si:?}");
        }
        for no in ["", "n", "no", "quizá", "\r\n"] {
            assert!(!es_si(no), "{no:?}");
        }
    }

    #[test]
    fn codifica_para_la_ruta() {
        assert_eq!(codificar("qwen3-4b_q4.k"), "qwen3-4b_q4.k");
        assert_eq!(codificar("refs/pr/1"), "refs%2Fpr%2F1");
        assert_eq!(codificar("ñ"), "%C3%B1");
    }
}
