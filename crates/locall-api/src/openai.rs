//! Compatibilidad con la API de **OpenAI** (lo que esperan herramientas como Continue, Open WebUI o los SDK
//! oficiales): `GET /v1/models` y `POST /v1/chat/completions`.
//!
//! - La "API key" es el token de Locall (`Authorization: Bearer …`, como envían los SDK).
//! - `model` es el identificador del modelo en Locall (el que da `GET /v1/models`).
//! - Con `"stream": true` la respuesta va por **SSE** (`data: {...}` + línea en blanco), terminada en
//!   `data: [DONE]`. Sin `stream` (por defecto en OpenAI) llega entera.
//!
//! **Lo que NO se admite todavía** se rechaza con un error claro en vez de ignorarlo en silencio: herramientas
//! (`tools`, `functions`), varias respuestas (`n` > 1), formato forzado (`response_format`) e imágenes en los
//! mensajes. Los parámetros de muestreo que el motor no recibe de Locall (`top_p`, `presence_penalty`…) se
//! ignoran (ADR 0016).

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use locall_core::ErrorNucleo;
use locall_engine::{Evento, Fin, Mensaje, PeticionChat, Resumen, Rol};
use serde_json::{Value, json};

use crate::flujo::{self, Paso};
use crate::{Compartido, ErrorApi, en_hilo};

fn invalida(mensaje: impl Into<String>) -> ErrorApi {
    ErrorApi::new(StatusCode::BAD_REQUEST, "invalid_request", mensaje)
}

// ─── GET /v1/models ───────────────────────────────────────────────────────────

pub(crate) async fn modelos(State(c): State<Compartido>) -> Result<Json<Value>, ErrorApi> {
    let modelos = en_hilo(move || c.nucleo.modelos()).await?;
    let datos: Vec<Value> = modelos
        .iter()
        .map(|m| {
            json!({
                "id": m.id.to_string(),
                "object": "model",
                "created": m.instalado,
                "owned_by": "locall",
            })
        })
        .collect();
    Ok(Json(json!({"object": "list", "data": datos})))
}

// ─── POST /v1/chat/completions ────────────────────────────────────────────────

pub(crate) struct Peticion {
    modelo: String,
    chat: PeticionChat,
    en_trozos: bool,
    incluir_uso: bool,
}

pub(crate) async fn completar(
    State(c): State<Compartido>,
    cuerpo: Bytes,
) -> Result<Response, ErrorApi> {
    let p = leer_peticion(&cuerpo)?;
    let nucleo = Arc::clone(&c.nucleo);
    let buscado = p.modelo.clone();
    if en_hilo(move || nucleo.modelo(&buscado)).await?.is_none() {
        return Err(ErrorNucleo::NoInstalado(p.modelo).into());
    }

    let id = format!("chatcmpl-{}", aleatorio());
    let creado = locall_registry::ahora();
    let (emisor, rx) = flujo::canal::<Resumen>();
    let nucleo = Arc::clone(&c.nucleo);
    let (modelo, peticion) = (p.modelo.clone(), p.chat);
    tokio::task::spawn_blocking(move || {
        let cancelar = Arc::clone(&emisor.cancelar);
        let resultado = nucleo.chat(&modelo, &peticion, &cancelar, |evento| match evento {
            // OpenAI no tiene eventos de "leyendo el prompt": no se envían.
            Evento::Leyendo { .. } => {}
            Evento::Razonamiento(t) => emisor.evento(json!({"reasoning_content": t})),
            Evento::Texto(t) => emisor.evento(json!({"content": t})),
        });
        emisor.fin(resultado);
    });

    if !p.en_trozos {
        let r = flujo::esperar_fin(rx).await?;
        let mut mensaje = json!({"role": "assistant", "content": r.texto});
        if !r.razonamiento.is_empty() {
            mensaje["reasoning_content"] = json!(r.razonamiento);
        }
        return Ok(Json(json!({
            "id": id,
            "object": "chat.completion",
            "created": creado,
            "model": p.modelo,
            "choices": [{"index": 0, "message": mensaje, "finish_reason": razon(r.fin)}],
            "usage": uso(&r),
        }))
        .into_response());
    }

    // Por trozos (SSE). El primer trozo anuncia el rol, como hace OpenAI.
    let modelo = p.modelo;
    let incluir_uso = p.incluir_uso;
    let mut primero = true;
    let trozo = move |delta: Value, razon_fin: Value| {
        json!({
            "id": id,
            "object": "chat.completion.chunk",
            "created": creado,
            "model": modelo,
            "choices": [{"index": 0, "delta": delta, "finish_reason": razon_fin}],
        })
    };
    flujo::transmitir(rx, "text/event-stream; charset=utf-8", move |paso| {
        let mut salida = String::new();
        let mut enviar = |v: Value| salida.push_str(&format!("data: {v}\n\n"));
        if primero {
            primero = false;
            enviar(trozo(
                json!({"role": "assistant", "content": ""}),
                Value::Null,
            ));
        }
        match paso {
            Paso::Evento(delta) => enviar(trozo(delta, Value::Null)),
            Paso::Fin(Ok(r)) => {
                enviar(trozo(json!({}), json!(razon(r.fin))));
                if incluir_uso {
                    let mut final_uso = trozo(json!({}), Value::Null);
                    final_uso["choices"] = json!([]);
                    final_uso["usage"] = uso(&r);
                    enviar(final_uso);
                }
                salida.push_str("data: [DONE]\n\n");
            }
            Paso::Fin(Err(e)) => enviar(json!({"error": ErrorApi::from(e).cuerpo()})),
        }
        Some(salida)
    })
    .await
}

fn razon(fin: Fin) -> &'static str {
    match fin {
        Fin::Limite => "length",
        // OpenAI no tiene "cancelado" (si el cliente se va, ya no lo lee): "stop".
        Fin::Completo | Fin::Cancelado => "stop",
    }
}

fn uso(r: &Resumen) -> Value {
    let entrada = r.tokens_leidos + r.tokens_cache;
    json!({
        "prompt_tokens": entrada,
        "completion_tokens": r.tokens_generados,
        "total_tokens": entrada + r.tokens_generados,
    })
}

fn aleatorio() -> String {
    let mut bytes = [0u8; 12];
    let _ = getrandom::fill(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn leer_peticion(cuerpo: &[u8]) -> Result<Peticion, ErrorApi> {
    let json: Value = serde_json::from_slice(cuerpo)
        .map_err(|e| invalida(format!("el cuerpo no es JSON válido: {e}")))?;

    // Lo que no se admite todavía: mejor un error claro que ignorarlo y que la herramienta no entienda qué pasa.
    for campo in [
        "tools",
        "functions",
        "tool_choice",
        "function_call",
        "response_format",
    ] {
        if !json[campo].is_null() {
            return Err(invalida(format!("Locall todavía no admite «{campo}»")));
        }
    }
    if json["n"].as_u64().is_some_and(|n| n != 1) {
        return Err(invalida(
            "Locall solo genera una respuesta por petición («n» = 1)",
        ));
    }

    let modelo = json["model"]
        .as_str()
        .filter(|m| !m.is_empty())
        .ok_or_else(|| invalida("falta «model»"))?
        .to_owned();
    let lista = json["messages"]
        .as_array()
        .filter(|l| !l.is_empty())
        .ok_or_else(|| invalida("«messages» debe ser una lista con al menos un mensaje"))?;
    let mut mensajes = Vec::with_capacity(lista.len());
    for (i, m) in lista.iter().enumerate() {
        let rol = match m["role"].as_str() {
            // "developer" es el nombre nuevo de "system" en la API de OpenAI.
            Some("system" | "developer") => Rol::Sistema,
            Some("user") => Rol::Usuario,
            Some("assistant") => Rol::Asistente,
            Some(otro) => {
                return Err(invalida(format!(
                    "mensaje {i}: el rol «{otro}» no se admite"
                )));
            }
            None => return Err(invalida(format!("mensaje {i}: falta «role»"))),
        };
        mensajes.push(Mensaje::nuevo(rol, contenido(&m["content"], i)?));
    }

    let max_tokens = match (&json["max_completion_tokens"], &json["max_tokens"]) {
        (Value::Null, Value::Null) => None,
        (v, Value::Null) | (Value::Null, v) | (v, _) => Some(
            v.as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .filter(|&n| n > 0)
                .ok_or_else(|| invalida("«max_tokens» debe ser un número positivo"))?,
        ),
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
    let semilla = match &json["seed"] {
        Value::Null => None,
        v => Some(
            v.as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| invalida("«seed» debe ser un número entero positivo"))?,
        ),
    };
    let en_trozos = match &json["stream"] {
        Value::Null => false, // en OpenAI, por defecto, la respuesta va entera
        Value::Bool(b) => *b,
        _ => return Err(invalida("«stream» debe ser true o false")),
    };
    let incluir_uso = json["stream_options"]["include_usage"].as_bool() == Some(true);
    Ok(Peticion {
        modelo,
        chat: PeticionChat {
            mensajes,
            max_tokens,
            temperatura,
            semilla,
        },
        en_trozos,
        incluir_uso,
    })
}

/// El contenido de un mensaje: un texto, o una lista de partes `{"type": "text", "text": "..."}` (formato nuevo
/// de OpenAI). Las imágenes no se admiten todavía.
fn contenido(valor: &Value, i: usize) -> Result<String, ErrorApi> {
    match valor {
        Value::String(t) => Ok(t.clone()),
        Value::Array(partes) => {
            let mut texto = String::new();
            for parte in partes {
                match parte["type"].as_str() {
                    Some("text") => texto.push_str(parte["text"].as_str().unwrap_or("")),
                    Some(otro) => {
                        return Err(invalida(format!(
                            "mensaje {i}: Locall todavía no admite contenido de tipo «{otro}»"
                        )));
                    }
                    None => return Err(invalida(format!("mensaje {i}: parte sin «type»"))),
                }
            }
            Ok(texto)
        }
        _ => Err(invalida(format!("mensaje {i}: falta «content» (texto)"))),
    }
}
