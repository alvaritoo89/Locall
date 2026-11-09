//! # locall-cli
//!
//! La línea de comandos de Locall (Fase 4). Es un **cliente de la API local**, igual que lo será la app de
//! escritorio (regla 9): nunca toca el núcleo directamente. Si Locall no está abierto, lo arranca en segundo plano
//! (ADR 0017).

use std::path::Path;
use std::time::{Duration, Instant};

use locall_config::Rutas;
use serde_json::Value;

pub mod arranque;
pub mod cliente;
pub mod formato;
pub mod ordenes;

pub use cliente::{Cliente, ErrorCli};
pub use ordenes::{AYUDA, Orden, interpretar};

/// Ejecuta una orden (todas menos `serve`, que la atiende el programa). `ejecutable` es `locall.exe`, para poder
/// arrancar el núcleo en segundo plano. Devuelve el texto para el usuario.
pub fn ejecutar(orden: &Orden, rutas: &Rutas, ejecutable: &Path) -> Result<String, ErrorCli> {
    match orden {
        Orden::Ayuda => Ok(AYUDA.to_owned()),
        Orden::Version => Ok(format!("locall {}\n", env!("CARGO_PKG_VERSION"))),
        Orden::Servir => Err(ErrorCli::Uso("«serve» lo atiende el programa".into())),
        Orden::Estado => estado(rutas),
        Orden::Parar => parar(rutas),
        Orden::Lista => lista(&arranque::conectar_o_arrancar(rutas, ejecutable)?),
        Orden::Mostrar(id) => mostrar(&arranque::conectar_o_arrancar(rutas, ejecutable)?, id),
    }
}

/// `locall status`: no arranca Locall; solo dice cómo está.
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

/// `locall stop`: pide a Locall que se cierre y espera a que deje de responder.
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
    let m = cliente.get(&format!("/api/models/{}", codificar(id)))?;
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

/// Un texto del JSON, o «—» si no hay.
fn texto(valor: &Value) -> String {
    valor.as_str().map_or_else(|| "—".to_owned(), str::to_owned)
}

/// Lo justo para meter un identificador en una ruta (los válidos solo llevan letras, números, `-`, `_` y `.`).
fn codificar(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c.to_string()
            } else {
                format!("%{:02X}", c as u32 & 0xFF)
            }
        })
        .collect()
}
