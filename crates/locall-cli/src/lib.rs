//! # locall-cli
//!
//! Locall's command line (Phase 4). It is a **client of the local API**, just like the desktop app (rule 9): it never
//! touches the core directly. If Locall is not open, it starts it in the background (ADR 0017).

use std::path::Path;
use std::time::{Duration, Instant};

use locall_config::Rutas;
use serde_json::Value;

pub mod arranque;
pub mod chat;
pub mod cliente;
pub mod formato;
pub mod modelos;
pub mod ordenes;
pub mod progreso;

pub use cliente::{Cliente, ErrorCli};
pub use ordenes::{AYUDA, Orden, interpretar};

/// Runs a command (all except `serve`, which the program handles). `ejecutable` is `locall.exe`, so the core can be
/// started in the background. Returns the text for the user.
pub fn ejecutar(orden: &Orden, rutas: &Rutas, ejecutable: &Path) -> Result<String, ErrorCli> {
    match orden {
        Orden::Ayuda => Ok(AYUDA.to_owned()),
        Orden::Version => Ok(format!("locall {}\n", env!("CARGO_PKG_VERSION"))),
        Orden::Servir => Err(ErrorCli::Uso("«serve» lo atiende el programa".into())),
        Orden::Estado => estado(rutas),
        Orden::Parar => parar(rutas),
        Orden::Lista => lista(&arranque::conectar_o_arrancar(rutas, ejecutable)?),
        Orden::Mostrar(id) => mostrar(&arranque::conectar_o_arrancar(rutas, ejecutable)?, id),
        Orden::Instalar {
            repo,
            archivo,
            revision,
            id,
        } => {
            let cliente = arranque::conectar_o_arrancar(rutas, ejecutable)?;
            match archivo {
                None => modelos::listar_repo(&cliente, repo, revision.as_deref()),
                Some(archivo) => {
                    modelos::instalar(&cliente, repo, archivo, revision.as_deref(), id.as_deref())
                }
            }
        }
        Orden::InstalarCatalogo(nombre) => modelos::instalar_del_catalogo(
            &arranque::conectar_o_arrancar(rutas, ejecutable)?,
            nombre,
        ),
        Orden::Catalogo => modelos::catalogo(&arranque::conectar_o_arrancar(rutas, ejecutable)?),
        Orden::Importar { ruta, id } => modelos::importar(
            &arranque::conectar_o_arrancar(rutas, ejecutable)?,
            ruta,
            id.as_deref(),
        ),
        Orden::Conversar { id, pregunta } => chat::ejecutar(
            &arranque::conectar_o_arrancar(rutas, ejecutable)?,
            id,
            pregunta.as_deref(),
        ),
        Orden::Eliminar { id, sin_preguntar } => modelos::eliminar(
            &arranque::conectar_o_arrancar(rutas, ejecutable)?,
            id,
            *sin_preguntar,
        ),
    }
}

/// `locall status`: does not start Locall; it only says how it is.
fn estado(rutas: &Rutas) -> Result<String, ErrorCli> {
    let Some(cliente) = arranque::conectar(rutas)? else {
        return Ok("Locall no está abierto. Se abrirá solo cuando lo uses (por ejemplo, con «locall list»).\n".into());
    };
    let version = cliente.get("/api/version")?;
    let estado = cliente.get("/api/status")?;
    let mut texto = format!(
        "Locall {} · {} · {}\n",
        version["version"].as_str().unwrap_or("?"),
        version["engine"].as_str().unwrap_or("?"),
        cliente.base()
    );
    let modelo = estado["model"].as_str().unwrap_or("");
    texto.push_str(&match estado["state"].as_str() {
        Some("loading") => format!("Cargando el modelo «{modelo}»…\n"),
        Some("ready") if estado["in_use"].as_u64().unwrap_or(0) > 0 => {
            format!("Modelo cargado: {modelo} (respondiendo)\n")
        }
        Some("ready") => format!("Modelo cargado: {modelo}\n"),
        _ => "Ningún modelo cargado\n".into(),
    });
    for aviso in estado["warnings"].as_array().into_iter().flatten() {
        texto.push_str(&format!("Aviso: {}\n", aviso.as_str().unwrap_or("")));
    }
    Ok(texto)
}

/// `locall stop`: asks Locall to close and waits until it stops answering.
fn parar(rutas: &Rutas) -> Result<String, ErrorCli> {
    let Some(cliente) = arranque::conectar(rutas)? else {
        return Ok("Locall no estaba abierto.\n".into());
    };
    cliente.post("/api/shutdown", &Value::Null)?;
    let inicio = Instant::now();
    while cliente.responde() {
        if inicio.elapsed() > Duration::from_secs(15) {
            return Err(ErrorCli::Arranque(
                "pidió cerrarse pero sigue respondiendo (¿hay una respuesta muy larga en curso?)"
                    .into(),
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok("Locall cerrado.\n".into())
}

/// `locall list`
fn lista(cliente: &Cliente) -> Result<String, ErrorCli> {
    let respuesta = cliente.get("/api/models")?;
    let modelos = respuesta["models"].as_array().cloned().unwrap_or_default();
    if modelos.is_empty() {
        return Ok("No hay modelos instalados.\n\
                   Instala uno con: locall pull <repositorio> <archivo.gguf>\n"
            .into());
    }
    let ahora = formato::ahora();
    let filas: Vec<Vec<String>> = modelos
        .iter()
        .map(|m| {
            vec![
                texto(&m["id"]),
                formato::tamano(m["size_bytes"].as_u64().unwrap_or(0)),
                texto(&m["quantization"]),
                texto(&m["source"]),
                formato::hace(m["last_used_at"].as_u64(), ahora),
            ]
        })
        .collect();
    Ok(formato::tabla(
        &["MODELO", "TAMAÑO", "CUANTIZACIÓN", "ORIGEN", "ÚLTIMO USO"],
        &filas,
    ))
}

/// `locall show <id>`
fn mostrar(cliente: &Cliente, id: &str) -> Result<String, ErrorCli> {
    let m = cliente.get(&format!("/api/models/{}", modelos::codificar(id)))?;
    let ahora = formato::ahora();
    let mut texto_salida = String::new();
    let mut linea = |nombre: &str, valor: String| {
        texto_salida.push_str(&format!("{nombre:<15}{valor}\n"));
    };
    linea("Modelo", texto(&m["id"]));
    linea("Nombre", texto(&m["name"]));
    linea("Origen", texto(&m["source"]));
    linea("Revisión", texto(&m["revision"]));
    linea("Arquitectura", texto(&m["architecture"]));
    linea("Cuantización", texto(&m["quantization"]));
    linea(
        "Contexto máx.",
        m["context_max"]
            .as_u64()
            .map_or_else(|| "—".into(), |n| format!("{n} tokens")),
    );
    linea("Licencia", texto(&m["license"]));
    linea(
        "Tamaño",
        formato::tamano(m["size_bytes"].as_u64().unwrap_or(0)),
    );
    linea(
        "Instalado",
        formato::hace(m["installed_at"].as_u64(), ahora),
    );
    linea(
        "Último uso",
        formato::hace(m["last_used_at"].as_u64(), ahora),
    );
    for archivo in m["files"].as_array().into_iter().flatten() {
        linea(
            "Archivo",
            format!(
                "{} ({}, sha256 {})",
                texto(&archivo["name"]),
                formato::tamano(archivo["size_bytes"].as_u64().unwrap_or(0)),
                texto(&archivo["sha256"])
            ),
        );
    }
    Ok(texto_salida)
}

/// A text from the JSON, or «—» if there is none.
fn texto(valor: &Value) -> String {
    valor.as_str().map_or_else(|| "—".to_owned(), str::to_owned)
}
