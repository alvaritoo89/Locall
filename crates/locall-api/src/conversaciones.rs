//! Saved conversations (Phase 5d). The core stores them, so the app and the CLI see the same ones.
//!
//! | Route | What it does |
//! |------|----------|
//! | `GET /api/conversations` | Lists them, from the most recent to the oldest (without messages). |
//! | `POST /api/conversations` | Creates an empty one. Optional body: `{"title": "..."}`. |
//! | `GET /api/conversations/{id}` | A conversation with its messages. |
//! | `PATCH /api/conversations/{id}` | Changes the title: `{"title": "..."}`. |
//! | `DELETE /api/conversations/{id}` | Deletes it **for real**, also from disk (see `locall-registry`). |
//! | `POST /api/conversations/{id}/messages` | Writes a message and saves the model's answer. |
//!
//! The `messages` body is `{"content": "...", "model": "...", "system": "...", "temperature": 0.7}` (`system` and
//! `temperature` optional). The **whole** saved conversation is sent to the model. The response is NDJSON, like
//! `/api/chat`, and the last event also carries the saved conversation and message:
//!
//! ```text
//! {"type":"text","text":"Hola"}
//! {"type":"done","finish":"stop","usage":{...},"speed":{...},"conversation":{...},"message":{...}}
//! ```
//!
//! If the model fails or is stopped before answering anything, the question is **not** saved (`"message": null`).

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use locall_core::RespuestaGuardada;
use locall_registry::{Autor, Conversacion, MensajeGuardado};
use serde_json::{Value, json};

use crate::{Compartido, ErrorApi, chat, en_hilo, flujo};

fn invalida(mensaje: impl Into<String>) -> ErrorApi {
    ErrorApi::new(StatusCode::BAD_REQUEST, "invalid_request", mensaje)
}

/// The identifier from the route. (Read by hand so a malformed id gives an error in our format.)
fn id_de(texto: &str) -> Result<i64, ErrorApi> {
    texto
        .parse::<i64>()
        .ok()
        .filter(|&id| id > 0)
        .ok_or_else(|| invalida(format!("«{texto}» no es un identificador de conversación")))
}

/// The body as JSON; an empty body counts as `{}`.
fn json_de(cuerpo: &[u8]) -> Result<Value, ErrorApi> {
    if cuerpo.iter().all(u8::is_ascii_whitespace) {
        return Ok(json!({}));
    }
    let json: Value = serde_json::from_slice(cuerpo)
        .map_err(|e| invalida(format!("el cuerpo no es JSON válido: {e}")))?;
    if !json.is_object() {
        return Err(invalida("el cuerpo debe ser un objeto JSON"));
    }
    Ok(json)
}

fn titulo_de(json: &Value, obligatorio: bool) -> Result<String, ErrorApi> {
    match &json["title"] {
        Value::Null if !obligatorio => Ok(String::new()),
        Value::String(t) if !obligatorio || !t.trim().is_empty() => Ok(t.clone()),
        _ => Err(invalida("«title» debe ser un texto no vacío")),
    }
}

pub(crate) async fn listar(State(c): State<Compartido>) -> Result<Json<Value>, ErrorApi> {
    let lista = en_hilo(move || c.nucleo.conversaciones()).await?;
    Ok(Json(json!({
        "conversations": lista.iter().map(conversacion_json).collect::<Vec<_>>()
    })))
}

pub(crate) async fn crear(
    State(c): State<Compartido>,
    cuerpo: Bytes,
) -> Result<(StatusCode, Json<Value>), ErrorApi> {
    let titulo = titulo_de(&json_de(&cuerpo)?, false)?;
    let nueva = en_hilo(move || c.nucleo.crear_conversacion(&titulo)).await?;
    Ok((StatusCode::CREATED, Json(conversacion_json(&nueva))))
}

pub(crate) async fn abrir(
    State(c): State<Compartido>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ErrorApi> {
    let id = id_de(&id)?;
    let (conversacion, mensajes) = en_hilo(move || c.nucleo.conversacion(id)).await?;
    let mut json = conversacion_json(&conversacion);
    json["messages"] = mensajes.iter().map(mensaje_json).collect();
    Ok(Json(json))
}

pub(crate) async fn renombrar(
    State(c): State<Compartido>,
    Path(id): Path<String>,
    cuerpo: Bytes,
) -> Result<Json<Value>, ErrorApi> {
    let id = id_de(&id)?;
    let titulo = titulo_de(&json_de(&cuerpo)?, true)?;
    let cambiada = en_hilo(move || {
        c.nucleo.renombrar_conversacion(id, &titulo)?;
        Ok(c.nucleo.conversacion(id)?.0)
    })
    .await?;
    Ok(Json(conversacion_json(&cambiada)))
}

pub(crate) async fn eliminar(
    State(c): State<Compartido>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ErrorApi> {
    let id = id_de(&id)?;
    en_hilo(move || c.nucleo.eliminar_conversacion(id)).await?;
    Ok(Json(json!({ "deleted": id })))
}

/// What `POST /api/conversations/{id}/messages` asks for.
pub(crate) struct Peticion {
    contenido: String,
    modelo: String,
    sistema: String,
    temperatura: Option<f32>,
    en_trozos: bool,
}

pub(crate) fn leer_peticion(cuerpo: &[u8]) -> Result<Peticion, ErrorApi> {
    let json = json_de(cuerpo)?;
    let contenido = json["content"]
        .as_str()
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| invalida("falta «content» (el mensaje, un texto no vacío)"))?
        .to_owned();
    let modelo = json["model"]
        .as_str()
        .filter(|m| !m.is_empty())
        .ok_or_else(|| invalida("falta «model»"))?
        .to_owned();
    let sistema = match &json["system"] {
        Value::Null => String::new(),
        Value::String(s) => s.trim().to_owned(),
        _ => return Err(invalida("«system» debe ser un texto")),
    };
    let temperatura = match &json["temperature"] {
        Value::Null => None,
        v => Some(
            v.as_f64()
                .filter(|t| (0.0..=2.0).contains(t))
                .ok_or_else(|| invalida("«temperature» debe estar entre 0 y 2"))?
                as f32,
        ),
    };
    let en_trozos = match &json["stream"] {
        Value::Null => true,
        Value::Bool(b) => *b,
        _ => return Err(invalida("«stream» debe ser true o false")),
    };
    Ok(Peticion {
        contenido,
        modelo,
        sistema,
        temperatura,
        en_trozos,
    })
}

pub(crate) async fn escribir(
    State(c): State<Compartido>,
    Path(id): Path<String>,
    cuerpo: Bytes,
) -> Result<Response, ErrorApi> {
    let id = id_de(&id)?;
    let p = leer_peticion(&cuerpo)?;
    let en_trozos = p.en_trozos;

    // Same as /api/chat: the core works on a separate thread and sends the events through a channel. If the
    // conversation or the model does not exist, it fails before the first event and the response is a normal HTTP
    // error (404).
    let (emisor, rx) = flujo::canal::<RespuestaGuardada>();
    let nucleo = Arc::clone(&c.nucleo);
    tokio::task::spawn_blocking(move || {
        let cancelar = Arc::clone(&emisor.cancelar);
        let resultado = nucleo.conversar(
            id,
            &p.contenido,
            &p.modelo,
            &p.sistema,
            p.temperatura,
            &cancelar,
            |evento| emisor.evento(chat::evento_json(&evento)),
        );
        emisor.fin(resultado);
    });

    if en_trozos {
        flujo::en_trozos(rx, fin_json).await
    } else {
        let r = flujo::esperar_fin(rx).await?;
        let mut json = fin_json(&r);
        if let Some(objeto) = json.as_object_mut() {
            objeto.remove("type");
        }
        Ok(Json(json).into_response())
    }
}

fn fin_json(r: &RespuestaGuardada) -> Value {
    let mut json = chat::fin_json(&r.resumen);
    json["conversation"] = conversacion_json(&r.conversacion);
    json["message"] = r.respuesta.as_ref().map_or(Value::Null, mensaje_json);
    json
}

/// A conversation as JSON (without the messages).
fn conversacion_json(c: &Conversacion) -> Value {
    json!({
        "id": c.id,
        "title": c.titulo,
        "model": c.modelo,
        "system": c.sistema,
        "temperature": c.temperatura,
        "created_at": c.creada,
        "updated_at": c.actualizada,
        "message_count": c.mensajes,
    })
}

/// A saved message as JSON.
fn mensaje_json(m: &MensajeGuardado) -> Value {
    json!({
        "index": m.orden,
        "role": match m.autor {
            Autor::Usuario => "user",
            Autor::Asistente => "assistant",
        },
        "content": m.contenido,
        "reasoning": m.razonamiento,
        "model": m.modelo,
        "completion_tokens": m.tokens,
        "generation_per_second": m.velocidad,
        "truncated": m.cortada,
        "created_at": m.creado,
    })
}
