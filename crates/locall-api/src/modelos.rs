//! Model management through the API.
//!
//! - `GET /api/hub/{owner}/{name}?revision=main`: which GGUFs a Hugging Face repository has (pinned to the commit).
//! - `POST /api/models/pull`: installs one, with chunked progress (NDJSON). If the client disconnects, the download
//!   is paused and next time it **continues** where it left off.
//! - `POST /api/models/import`: brings in a GGUF that is already on disk.
//! - `DELETE /api/models/{id}`: removes a model and frees what nobody else uses.

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

/// At most how often a progress event is sent (a 5 GB download would give 20,000 otherwise).
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

// ─── Querying a repository ────────────────────────────────────────────────────

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

// ─── Searching Hugging Face ───────────────────────────────────────────────────

/// `POST /api/hub/search` with `{"query": "qwen", "limit": 20}` (`limit` optional, 1 to 50; 20 if not given).
///
/// It is `POST` with the text in the body (and not `GET ?q=…`) so what is searched never becomes part of an address.
pub(crate) async fn buscar(
    State(c): State<Compartido>,
    cuerpo: Bytes,
) -> Result<Json<Value>, ErrorApi> {
    let json = leer_json(&cuerpo)?;
    let texto = texto_obligatorio(&json, "query")?;
    let limite = match &json["limit"] {
        Value::Null => 20,
        v => v
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| (1..=locall_core::BUSQUEDA_MAX).contains(n))
            .ok_or_else(|| {
                ErrorApi::new(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    format!(
                        "«limit» debe ser un número de 1 a {}",
                        locall_core::BUSQUEDA_MAX
                    ),
                )
            })?,
    };
    let encontrados = en_hilo(move || c.nucleo.buscar(&texto, limite)).await?;
    Ok(Json(json!({
        "results": encontrados.iter().map(|r| json!({
            "repo": r.repo,
            "downloads": r.descargas,
            "likes": r.me_gusta,
            "license": r.licencia,
            "task": r.tarea,
        })).collect::<Vec<_>>(),
    })))
}

// ─── Installing ───────────────────────────────────────────────────────────────

/// What is requested to install.
enum QueInstalar {
    Repo(PeticionInstalar),
    /// A catalog name (`qwen3-4b` or `qwen3-4b:q8_0`).
    Catalogo(String),
}

pub(crate) async fn instalar(
    State(c): State<Compartido>,
    cuerpo: Bytes,
) -> Result<Response, ErrorApi> {
    let json = leer_json(&cuerpo)?;
    // From the catalog (`{"name": "qwen3-4b"}`) or from Hugging Face (`{"repo": …, "file": …}`).
    let que = match texto_opcional(&json, "name")? {
        Some(nombre) => {
            if json.get("repo").is_some() || json.get("file").is_some() {
                return Err(invalida(
                    "usa «name» (del catálogo) o «repo» y «file» (de Hugging Face), no los dos",
                ));
            }
            QueInstalar::Catalogo(nombre)
        }
        None => QueInstalar::Repo(PeticionInstalar {
            repo: texto_obligatorio(&json, "repo")?,
            revision: texto_opcional(&json, "revision")?.unwrap_or_else(|| "main".to_owned()),
            archivo: texto_obligatorio(&json, "file")?,
            id: texto_opcional(&json, "id")?,
        }),
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
        let progreso = |p: ProgresoInstalacion| {
            // At most every 250 ms, but always the first and the last of each part.
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
        };
        let resultado = match &que {
            QueInstalar::Repo(peticion) => nucleo.instalar(peticion, &cancelar, progreso),
            QueInstalar::Catalogo(nombre) => {
                nucleo.instalar_del_catalogo(nombre, &cancelar, progreso)
            }
        };
        emisor.fin(resultado);
    });

    if en_trozos {
        flujo::en_trozos(rx, |m| json!({"type": "done", "model": modelo_json(m)})).await
    } else {
        let modelo = flujo::esperar_fin(rx).await?;
        Ok(Json(modelo_json(&modelo)).into_response())
    }
}

// ─── Importing ────────────────────────────────────────────────────────────────

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

// ─── Removing ─────────────────────────────────────────────────────────────────

pub(crate) async fn eliminar(
    State(c): State<Compartido>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ErrorApi> {
    let borrado = id.clone();
    let liberados = en_hilo(move || c.nucleo.eliminar(&borrado)).await?;
    Ok(Json(json!({"deleted": id, "freed_bytes": liberados})))
}
