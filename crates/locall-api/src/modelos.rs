//! Gestión de modelos por la API.
//!
//! - `GET /api/hub/{owner}/{name}?revision=main`: qué GGUF tiene un repositorio de Hugging Face (fijado al commit).
//! - `POST /api/models/pull`: instala uno, con el progreso por trozos (NDJSON). Si el cliente se desconecta, la
//!   descarga se pausa y la próxima vez **continúa** donde se quedó.
//! - `POST /api/models/import`: incorpora un GGUF que ya está en el disco.
//! - `DELETE /api/models/{id}`: elimina un modelo y libera lo que nadie más usa.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use locall_core::{PeticionInstalar, ProgresoInstalacion};
use locall_registry::Modelo;
use serde_json::{Value, json};

use crate::{Compartido, ErrorApi, en_hilo, flujo, modelo_json};

/// Cada cuánto se envía, como mucho, un evento de progreso (una descarga de 5 GB daría 20 000 si no).
const INTERVALO_PROGRESO: Duration = Duration::from_millis(250);

fn invalida(mensaje: impl Into<String>) -> ErrorApi {
    ErrorApi::new(StatusCode::BAD_REQUEST, "invalid_request", mensaje)
}

fn leer_json(cuerpo: &[u8]) -> Result<Value, ErrorApi> {
    serde_json::from_slice(cuerpo)
        .map_err(|e| invalida(format!("el cuerpo no es JSON válido: {e}")))
}

fn texto_opcional(json: &Value, campo: &str) -> Result<Option<String>, ErrorApi> {
    match &json[campo] {
        Value::Null => Ok(None),
        Value::String(t) if !t.is_empty() => Ok(Some(t.clone())),
        _ => Err(invalida(format!("«{campo}» debe ser un texto"))),
    }
}

fn texto_obligatorio(json: &Value, campo: &str) -> Result<String, ErrorApi> {
    texto_opcional(json, campo)?.ok_or_else(|| invalida(format!("falta «{campo}»")))
}

// ─── Consultar un repositorio ────────────────────────────────────────────────

pub(crate) async fn hub(
    State(c): State<Compartido>,
    Path((dueno, nombre)): Path<(String, String)>,
    RawQuery(consulta): RawQuery,
) -> Result<Json<Value>, ErrorApi> {
    let revision = consulta
        .as_deref()
        .unwrap_or("")
        .split('&')
        .find_map(|par| par.strip_prefix("revision="))
        .map(|r| r.replace("%2F", "/").replace("%2f", "/"))
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| "main".to_owned());
    let repo = format!("{dueno}/{nombre}");
    let consultado = repo.clone();
    let (info, ggufs) = en_hilo(move || c.nucleo.consultar(&consultado, &revision)).await?;
    Ok(Json(json!({
        "repo": repo,
        "commit": info.commit,
        "license": info.licencia,
        "gated": info.protegido,
        "files": ggufs.iter().map(|a| json!({
            "path": a.ruta,
            "size_bytes": a.bytes,
            "sha256": a.huella.as_ref().map(|h| h.hex()),
        })).collect::<Vec<_>>(),
    })))
}

// ─── Instalar ─────────────────────────────────────────────────────────────────

pub(crate) async fn instalar(
    State(c): State<Compartido>,
    cuerpo: Bytes,
) -> Result<Response, ErrorApi> {
    let json = leer_json(&cuerpo)?;
    let peticion = PeticionInstalar {
        repo: texto_obligatorio(&json, "repo")?,
        revision: texto_opcional(&json, "revision")?.unwrap_or_else(|| "main".to_owned()),
        archivo: texto_obligatorio(&json, "file")?,
        id: texto_opcional(&json, "id")?,
    };
    let en_trozos = match &json["stream"] {
        Value::Null => true,
        Value::Bool(b) => *b,
        _ => return Err(invalida("«stream» debe ser true o false")),
    };

    let (emisor, rx) = flujo::canal::<Modelo>();
    let nucleo = Arc::clone(&c.nucleo);
    tokio::task::spawn_blocking(move || {
        let cancelar = Arc::clone(&emisor.cancelar);
        let mut ultimo: Option<Instant> = None;
        let resultado = nucleo.instalar(&peticion, &cancelar, |p: ProgresoInstalacion| {
            // Como mucho cada 250 ms, pero siempre el primero y el último de cada parte.
            let toca = ultimo.is_none_or(|t| t.elapsed() >= INTERVALO_PROGRESO)
                || p.descargados == p.total;
            if toca {
                ultimo = Some(Instant::now());
                emisor.evento(json!({
                    "type": "progress",
                    "part": p.parte,
                    "parts": p.partes,
                    "downloaded_bytes": p.descargados,
                    "total_bytes": p.total,
                }));
            }
        });
        emisor.fin(resultado);
    });

    if en_trozos {
        flujo::en_trozos(rx, |m| json!({"type": "done", "model": modelo_json(m)})).await
    } else {
        let modelo = flujo::esperar_fin(rx).await?;
        Ok(Json(modelo_json(&modelo)).into_response())
    }
}

// ─── Importar ─────────────────────────────────────────────────────────────────

pub(crate) async fn importar(
    State(c): State<Compartido>,
    cuerpo: Bytes,
) -> Result<Json<Value>, ErrorApi> {
    let json = leer_json(&cuerpo)?;
    let ruta = PathBuf::from(texto_obligatorio(&json, "path")?);
    if !ruta.is_absolute() {
        return Err(invalida(
            "«path» debe ser una ruta completa, por ejemplo D:\\Modelos\\modelo.gguf",
        ));
    }
    if !ruta.is_file() {
        return Err(ErrorApi::new(
            StatusCode::NOT_FOUND,
            "file_not_found",
            "no existe ese archivo",
        ));
    }
    let id = texto_opcional(&json, "id")?;
    let modelo = en_hilo(move || c.nucleo.importar(&ruta, id.as_deref())).await?;
    Ok(Json(modelo_json(&modelo)))
}

// ─── Eliminar ─────────────────────────────────────────────────────────────────

pub(crate) async fn eliminar(
    State(c): State<Compartido>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ErrorApi> {
    let borrado = id.clone();
    let liberados = en_hilo(move || c.nucleo.eliminar(&borrado)).await?;
    Ok(Json(json!({"deleted": id, "freed_bytes": liberados})))
}
