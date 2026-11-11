//! Conversaciones guardadas (Fase 5d). Las guarda el núcleo, así que la app y la CLI ven las mismas.
//!
//! | Ruta | Qué hace |
//! |------|----------|
//! | `GET /api/conversations` | Lista, de la más reciente a la más antigua (sin los mensajes). |
//! | `POST /api/conversations` | Crea una vacía. Cuerpo opcional: `{"title": "..."}`. |
//! | `GET /api/conversations/{id}` | Una conversación con sus mensajes. |
//! | `PATCH /api/conversations/{id}` | Cambia el título: `{"title": "..."}`. |
//! | `DELETE /api/conversations/{id}` | La elimina **de verdad**, también del disco (ver `locall-registry`). |
//! | `POST /api/conversations/{id}/messages` | Escribe un mensaje y guarda la respuesta del modelo. |
//!
//! El cuerpo de `messages` es `{"content": "...", "model": "...", "system": "...", "temperature": 0.7}` (`system` y
//! `temperature` opcionales). Se envía al modelo **toda** la conversación guardada. La respuesta es NDJSON, como
//! `/api/chat`, y el último evento lleva además la conversación y el mensaje guardados:
//!
//! ```text
//! {"type":"text","text":"Hola"}
//! {"type":"done","finish":"stop","usage":{...},"speed":{...},"conversation":{...},"message":{...}}
//! ```
//!
//! Si el modelo falla o se para antes de responder nada, la pregunta **no** se queda guardada (`"message": null`).

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

/// El identificador de la ruta. (Se lee a mano para que un id mal escrito dé un error con nuestro formato.)
fn id_de(texto: &str) -> Result<i64, ErrorApi> {
    texto
        .parse::<i64>()
        .ok()
        .filter(|&id| id > 0)
        .ok_or_else(|| invalida(format!("«{texto}» no es un identificador de conversación")))
}

/// El cuerpo como JSON; un cuerpo vacío cuenta como `{}`.
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

/// Lo que pide `POST /api/conversations/{id}/messages`.
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

    // Igual que /api/chat: el núcleo trabaja en un hilo aparte y manda los eventos por un canal. Si la conversación
    // o el modelo no existen, falla antes del primer evento y la respuesta es un error HTTP normal (404).
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

/// Una conversación en JSON (sin los mensajes).
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

/// Un mensaje guardado en JSON.
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
