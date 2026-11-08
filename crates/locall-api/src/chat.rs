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

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use locall_core::ErrorNucleo;
use locall_engine::{Evento, Fin, Mensaje, PeticionChat, Resumen, Rol};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::{Compartido, ErrorApi};

/// Lo que el hilo del núcleo envía al servidor.
enum Mensajero {
    Evento(Value),
    Fin(Result<Resumen, ErrorNucleo>),
}

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

    // El núcleo trabaja en un hilo aparte y nos manda los eventos por un canal.
    let (tx, mut rx) = mpsc::channel::<Mensajero>(64);
    let cancelar = Arc::new(AtomicBool::new(false));

    // Vigilante: si el cliente se va (se suelta el receptor), cancelar. Funciona también mientras se espera turno.
    let vigilado = tx.clone();
    let bandera = Arc::clone(&cancelar);
    tokio::spawn(async move {
        vigilado.closed().await;
        bandera.store(true, Ordering::Relaxed);
    });

    let nucleo = Arc::clone(&c.nucleo);
    let bandera = Arc::clone(&cancelar);
    tokio::task::spawn_blocking(move || {
        let resultado = nucleo.chat(&modelo, &peticion, &bandera, |evento| {
            if let Some(json) = evento_json(&evento)
                && tx.blocking_send(Mensajero::Evento(json)).is_err()
            {
                bandera.store(true, Ordering::Relaxed); // el cliente se ha ido
            }
        });
        let _ = tx.blocking_send(Mensajero::Fin(resultado));
    });

    if !en_trozos {
        // Sin trozos: esperar al final y responder de una vez.
        loop {
            match rx.recv().await {
                Some(Mensajero::Evento(_)) => {}
                Some(Mensajero::Fin(Ok(resumen))) => {
                    return Ok(Json(respuesta_completa(&resumen)).into_response());
                }
                Some(Mensajero::Fin(Err(e))) => return Err(e.into()),
                None => return Err(error_interno()),
            }
        }
    }

    // Con trozos: el primer mensaje decide si respondemos con un error HTTP o empezamos a enviar.
    let primero = match rx.recv().await {
        Some(Mensajero::Fin(Err(e))) => return Err(e.into()),
        Some(m) => m,
        None => return Err(error_interno()),
    };
    let flujo = futures_util::stream::unfold(
        (Some(primero), rx, false),
        |(pendiente, mut rx, terminado)| async move {
            if terminado {
                return None;
            }
            let mensaje = match pendiente {
                Some(m) => m,
                None => rx.recv().await?,
            };
            let (linea, terminado) = match mensaje {
                Mensajero::Evento(json) => (json, false),
                Mensajero::Fin(Ok(resumen)) => (fin_json(&resumen), true),
                Mensajero::Fin(Err(e)) => {
                    let error = ErrorApi::from(e);
                    (json!({"type": "error", "error": error.cuerpo()}), true)
                }
            };
            Some((
                Ok::<_, Infallible>(format!("{linea}\n")),
                (None, rx, terminado),
            ))
        },
    );
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/x-ndjson; charset=utf-8")],
        Body::from_stream(flujo),
    )
        .into_response())
}

fn error_interno() -> ErrorApi {
    ErrorApi::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "error interno de Locall",
    )
}

fn invalida(mensaje: impl Into<String>) -> ErrorApi {
    ErrorApi::new(StatusCode::BAD_REQUEST, "invalid_request", mensaje)
}

/// Lee y valida la petición.
fn leer_peticion(cuerpo: &[u8]) -> Result<(String, PeticionChat, bool), ErrorApi> {
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
fn evento_json(evento: &Evento<'_>) -> Option<Value> {
    Some(match evento {
        Evento::Leyendo { procesados, total } => {
            json!({"type": "prompt_progress", "processed": procesados, "total": total})
        }
        Evento::Razonamiento(t) => json!({"type": "reasoning", "text": t}),
        Evento::Texto(t) => json!({"type": "text", "text": t}),
    })
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

fn fin_json(r: &Resumen) -> Value {
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
