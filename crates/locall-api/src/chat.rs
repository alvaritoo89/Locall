//! `POST /api/chat`: conversación con un modelo instalado.
//!
//! Petición:
//!
//! ```json
//! { "model": "qwen3-4b", "messages": [{"role": "user", "content": "Hola"}],
//!   "stream": true, "options": {"max_tokens": 200, "temperature": 0.7, "seed": 1} }
//! ```
//!
//! Con `"stream": true` (por defecto) la respuesta es **NDJSON**: una línea JSON por evento.
//!
//! ```text
//! {"type":"prompt_progress","processed":32,"total":32}
//! {"type":"text","text":"Hola"}
//! {"type":"done","finish":"stop","usage":{...},"speed":{...}}
//! ```
//!
//! Si el cliente **se desconecta**, se cancela: el motor deja de generar y, si aún esperaba turno, se va de la cola.
//!
//! Si algo falla antes del primer evento (modelo inexistente, no cabe en la memoria…), se responde con un error HTTP
//! normal. Si falla a mitad, llega un evento `{"type":"error","error":{...}}` y se cierra.

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use locall_core::ErrorNucleo;
use locall_engine::{Evento, Fin, Mensaje, PeticionChat, Resumen, Rol};
use serde_json::{Value, json};

use crate::{Compartido, ErrorApi, flujo};

pub(crate) async fn chat(State(c): State<Compartido>, cuerpo: Bytes) -> Result<Response, ErrorApi> {
    let (modelo, peticion, en_trozos) = leer_peticion(&cuerpo)?;

    // Comprobar que el modelo existe ANTES de empezar (así el error es un 404 normal).
    let nucleo = Arc::clone(&c.nucleo);
    let buscado = modelo.clone();
    if crate::en_hilo(move || nucleo.modelo(&buscado))
        .await?
        .is_none()
    {
        return Err(ErrorNucleo::NoInstalado(modelo).into());
    }

    // El núcleo trabaja en un hilo aparte y nos manda los eventos por un canal (ver `flujo`).
    let (emisor, rx) = flujo::canal::<Resumen>();
    let nucleo = Arc::clone(&c.nucleo);
    tokio::task::spawn_blocking(move || {
        let cancelar = Arc::clone(&emisor.cancelar);
        let resultado = nucleo.chat(&modelo, &peticion, &cancelar, |evento| {
            emisor.evento(evento_json(&evento));
        });
        emisor.fin(resultado);
    });

    if en_trozos {
        flujo::en_trozos(rx, fin_json).await
    } else {
        let resumen = flujo::esperar_fin(rx).await?;
        Ok(Json(respuesta_completa(&resumen)).into_response())
    }
}

fn invalida(mensaje: impl Into<String>) -> ErrorApi {
    ErrorApi::new(StatusCode::BAD_REQUEST, "invalid_request", mensaje)
}

/// Lee y valida la petición.
pub(crate) fn leer_peticion(cuerpo: &[u8]) -> Result<(String, PeticionChat, bool), ErrorApi> {
    let json: Value = serde_json::from_slice(cuerpo)
        .map_err(|e| invalida(format!("el cuerpo no es JSON válido: {e}")))?;
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
            Some("system") => Rol::Sistema,
            Some("user") => Rol::Usuario,
            Some("assistant") => Rol::Asistente,
            _ => {
                return Err(invalida(format!(
                    "mensaje {i}: «role» debe ser \"system\", \"user\" o \"assistant\""
                )));
            }
        };
        let contenido = m["content"]
            .as_str()
            .ok_or_else(|| invalida(format!("mensaje {i}: falta «content» (texto)")))?;
        mensajes.push(Mensaje::nuevo(rol, contenido));
    }
    let opciones = &json["options"];
    let max_tokens = match &opciones["max_tokens"] {
        Value::Null => None,
        v => Some(
            v.as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .filter(|&n| n > 0)
                .ok_or_else(|| invalida("«options.max_tokens» debe ser un número positivo"))?,
        ),
    };
    let temperatura = match &opciones["temperature"] {
        Value::Null => None,
        v => Some(
            v.as_f64()
                .filter(|t| (0.0..=2.0).contains(t))
                .ok_or_else(|| invalida("«options.temperature» debe estar entre 0 y 2"))?
                as f32,
        ),
    };
    let semilla = match &opciones["seed"] {
        Value::Null => None,
        v => Some(
            v.as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| invalida("«options.seed» debe ser un número entero positivo"))?,
        ),
    };
    let en_trozos = match &json["stream"] {
        Value::Null => true,
        Value::Bool(b) => *b,
        _ => return Err(invalida("«stream» debe ser true o false")),
    };
    Ok((
        modelo,
        PeticionChat {
            mensajes,
            max_tokens,
            temperatura,
            semilla,
        },
        en_trozos,
    ))
}

/// Un evento del motor en JSON (ya traducido por `locall-engine`: sin datos privados).
pub(crate) fn evento_json(evento: &Evento<'_>) -> Value {
    match evento {
        Evento::Leyendo { procesados, total } => {
            json!({"type": "prompt_progress", "processed": procesados, "total": total})
        }
        Evento::Razonamiento(t) => json!({"type": "reasoning", "text": t}),
        Evento::Texto(t) => json!({"type": "text", "text": t}),
    }
}

fn fin_texto(fin: Fin) -> &'static str {
    match fin {
        Fin::Completo => "stop",
        Fin::Limite => "length",
        Fin::Cancelado => "cancelled",
    }
}

fn uso_json(r: &Resumen) -> (Value, Value) {
    (
        json!({
            "prompt_tokens": r.tokens_leidos,
            "cached_tokens": r.tokens_cache,
            "completion_tokens": r.tokens_generados,
        }),
        json!({
            "prompt_per_second": r.lectura_por_segundo,
            "generation_per_second": r.escritura_por_segundo,
        }),
    )
}

pub(crate) fn fin_json(r: &Resumen) -> Value {
    let (uso, velocidad) = uso_json(r);
    json!({"type": "done", "finish": fin_texto(r.fin), "usage": uso, "speed": velocidad})
}

fn respuesta_completa(r: &Resumen) -> Value {
    let (uso, velocidad) = uso_json(r);
    json!({
        "message": {"role": "assistant", "content": r.texto, "reasoning": r.razonamiento},
        "finish": fin_texto(r.fin),
        "usage": uso,
        "speed": velocidad,
    })
}
