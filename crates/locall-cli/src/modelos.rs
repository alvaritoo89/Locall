//! `locall pull`, `locall import` y `locall rm`.

use std::io::{BufRead, BufReader, IsTerminal, Write};
use std::path::Path;

use serde_json::{Value, json};

use crate::cliente::{Cliente, ErrorCli};
use crate::formato;
use crate::progreso::Barra;

/// `locall pull <repo>`: lista los GGUF del repositorio para que el usuario elija.
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

/// `locall pull <repo> <archivo>`: instala con barra de progreso.
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
    let respuesta = cliente.post_sin_leer("/api/models/pull", &cuerpo)?;
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
                    "Instalado «{id}» ({}, verificado con sha256).\nDetalles: locall show {id}\n",
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

/// `locall import <ruta>`
pub fn importar(cliente: &Cliente, ruta: &str, id: Option<&str>) -> Result<String, ErrorCli> {
    // La API pide una ruta completa: la de la orden puede ser relativa a la carpeta actual.
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
         Detalles: locall show {id}\n",
        formato::tamano(m["size_bytes"].as_u64().unwrap_or(0)),
    ))
}

/// `locall rm <id>`: pide confirmación salvo con `--yes`.
pub fn eliminar(cliente: &Cliente, id: &str, sin_preguntar: bool) -> Result<String, ErrorCli> {
    // Primero, comprobar que existe (y saber cuánto ocupa) para poder preguntar con datos.
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

/// ¿Ha respondido que sí? (s, si, sí, y, yes; en mayúsculas o minúsculas).
pub fn es_si(respuesta: &str) -> bool {
    matches!(
        respuesta.trim().to_lowercase().as_str(),
        "s" | "si" | "sí" | "y" | "yes"
    )
}

/// Lo justo para meter un texto en una ruta de la API (los identificadores válidos no necesitan nada).
pub(crate) fn codificar(texto: &str) -> String {
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
