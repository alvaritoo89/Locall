//! Locall's catalog of compatible models (Phase 6d, ADR 0023).
//!
//! - `GET /api/catalog`: the catalog in use, with whether each model is installed and whether it fits on this
//!   computer (**approximate**: the model is not downloaded, so it is computed from the file size).
//! - `POST /api/catalog/update` with `{"force": false}` (optional): checks GitHub for a new one, at most once a day
//!   (unless `force`). Answers `{"result": "not_due" | "no_changes" | "updated", "catalog": {...}}`.
//!
//! Installing one: `POST /api/models/pull` with `{"name": "qwen3-4b"}` (see `modelos.rs`).

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use locall_core::{Actualizacion, EstadoCatalogo, Nucleo, OrigenCatalogo};
use serde_json::{Value, json};

use crate::{Compartido, ErrorApi, en_hilo};

pub(crate) async fn ver(State(c): State<Compartido>) -> Result<Json<Value>, ErrorApi> {
    let json = en_hilo(move || Ok(catalogo_json(&c.nucleo, &c.nucleo.catalogo()))).await?;
    Ok(Json(json))
}

pub(crate) async fn actualizar(
    State(c): State<Compartido>,
    cuerpo: Bytes,
) -> Result<Json<Value>, ErrorApi> {
    let forzar = if cuerpo.iter().all(u8::is_ascii_whitespace) {
        false
    } else {
        let json: Value = serde_json::from_slice(&cuerpo).map_err(|e| {
            ErrorApi::new(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                format!("el cuerpo no es JSON válido: {e}"),
            )
        })?;
        match &json["force"] {
            Value::Null => false,
            Value::Bool(b) => *b,
            _ => {
                return Err(ErrorApi::new(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "«force» debe ser true o false",
                ));
            }
        }
    };
    let json = en_hilo(move || {
        let resultado = c.nucleo.actualizar_catalogo(forzar)?;
        Ok(json!({
            "result": match resultado {
                Actualizacion::NoTocaba => "not_due",
                Actualizacion::SinNovedades => "no_changes",
                Actualizacion::Actualizado => "updated",
            },
            "catalog": catalogo_json(&c.nucleo, &c.nucleo.catalogo()),
        }))
    })
    .await?;
    Ok(Json(json))
}

fn catalogo_json(nucleo: &Nucleo, estado: &EstadoCatalogo) -> Value {
    let instalados: Vec<String> = nucleo
        .modelos()
        .map(|l| l.iter().map(|m| m.id.to_string()).collect())
        .unwrap_or_default();
    json!({
        "published": estado.catalogo.publicado,
        "source": match estado.origen {
            OrigenCatalogo::Incluido => "builtin",
            OrigenCatalogo::Descargado => "downloaded",
        },
        "checked_at": estado.comprobado,
        "models": estado.catalogo.modelos.iter().map(|m| {
            let v = m.por_defecto();
            let (estimado, encaje) = nucleo.encaje_aproximado(v.bytes());
            json!({
                "name": m.nombre,
                "title": m.titulo,
                "publisher": m.autor,
                "description": m.descripcion,
                "tags": m.etiquetas,
                "license": m.licencia,
                "context_length": m.contexto_max,
                // The only thing the catalog claims: the engine recognizes the file format (ADR 0022).
                "status": "compatible",
                "checked_engine": v.motor_comprobado,
                "repo": v.repo,
                "size_bytes": v.bytes(),
                "installed": instalados.contains(&m.nombre),
                "fit_estimate": crate::encaje_texto(encaje),
                "memory_estimate_bytes": estimado,
                "variants": m.variantes.iter().map(|v| json!({
                    "name": v.nombre,
                    "default": v.por_defecto,
                    "repo": v.repo,
                    "commit": v.commit,
                    "size_bytes": v.bytes(),
                })).collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>(),
    })
}
