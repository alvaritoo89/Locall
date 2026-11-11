//! Talking to the engine: chat request with a chunked answer (SSE), cancellation and translation.
//!
//! **Translation (D4):** `llama-server`'s answers include data that must not leave this module (for example, the
//! full model path in the `model` field). That is why they are never forwarded as is: only the fields we need are
//! read and returned with our own types ([`Evento`], [`Resumen`]).

use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};

use crate::{ErrorMotor, Motor};

// ─── Public types ─────────────────────────────────────────────────────────────

/// Who says a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rol {
    Sistema,
    Usuario,
    Asistente,
}

impl Rol {
    fn como_texto(self) -> &'static str {
        match self {
            Rol::Sistema => "system",
            Rol::Usuario => "user",
            Rol::Asistente => "assistant",
        }
    }
}

/// A message of the conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mensaje {
    pub rol: Rol,
    pub contenido: String,
}

impl Mensaje {
    pub fn nuevo(rol: Rol, contenido: impl Into<String>) -> Mensaje {
        Mensaje {
            rol,
            contenido: contenido.into(),
        }
    }
}

/// A chat request.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PeticionChat {
    pub mensajes: Vec<Mensaje>,
    /// Maximum tokens to generate (`None` = until the model finishes or the context is full).
    pub max_tokens: Option<u32>,
    /// Creativity (0 = always the most likely). `None` = the model's.
    pub temperatura: Option<f32>,
    /// Random seed, for repeatable answers.
    pub semilla: Option<u32>,
}

/// What keeps arriving while the engine works.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Evento<'a> {
    /// Reading the prompt: `procesados` of `total` tokens.
    Leyendo { procesados: u64, total: u64 },
    /// A chunk of the reasoning (models that "think", like Qwen3).
    Razonamiento(&'a str),
    /// A chunk of the answer.
    Texto(&'a str),
}

/// Why the answer ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fin {
    /// The model finished by itself.
    Completo,
    /// The maximum number of tokens was reached.
    Limite,
    /// It was cancelled on request.
    Cancelado,
}

/// Result of a complete answer (or a cancelled one: with what had time to be generated).
#[derive(Debug, Clone, PartialEq)]
pub struct Resumen {
    pub texto: String,
    pub razonamiento: String,
    pub fin: Fin,
    /// Prompt tokens read (not counting those reused from the cache).
    pub tokens_leidos: u64,
    /// Prompt tokens reused from an earlier request.
    pub tokens_cache: u64,
    pub tokens_generados: u64,
    /// Prompt reading speed (tokens/s).
    pub lectura_por_segundo: Option<f64>,
    /// Generation speed (tokens/s).
    pub escritura_por_segundo: Option<f64>,
}

// ─── Request ──────────────────────────────────────────────────────────────────

impl Motor {
    /// Requests an answer and delivers it in chunks to `al_recibir` while it is generated.
    ///
    /// If `cancelar` is set to `true`, the connection is cut: `llama-server` stops generating (checked in Phase 0) and
    /// what was generated so far is returned with [`Fin::Cancelado`]. Cancellation is honoured every time something
    /// arrives from the engine: a token, or a progress notice while it reads the prompt (one per batch).
    pub fn chat(
        &self,
        peticion: &PeticionChat,
        cancelar: &AtomicBool,
        mut al_recibir: impl FnMut(Evento<'_>),
    ) -> Result<Resumen, ErrorMotor> {
        let respuesta = self
            .agente
            .post(&format!("{}/v1/chat/completions", self.base_url()))
            .header("Authorization", &self.cabecera_clave())
            .header("Content-Type", "application/json")
            .send(cuerpo(peticion).to_string())
            .map_err(|e| ErrorMotor::Conexion(e.to_string()))?;

        let estado = respuesta.status().as_u16();
        if estado != 200 {
            let texto = respuesta.into_body().read_to_string().unwrap_or_default();
            return Err(error_de_respuesta(estado, &texto));
        }

        let mut resumen = Resumen {
            texto: String::new(),
            razonamiento: String::new(),
            fin: Fin::Completo,
            tokens_leidos: 0,
            tokens_cache: 0,
            tokens_generados: 0,
            lectura_por_segundo: None,
            escritura_por_segundo: None,
        };
        let lector = BufReader::new(respuesta.into_body().into_reader());
        for linea in lector.lines() {
            if cancelar.load(Ordering::Relaxed) {
                // On leaving, the reader is dropped and the connection closed: the engine stops generating.
                resumen.fin = Fin::Cancelado;
                log::info!("respuesta cancelada a petición");
                return Ok(resumen);
            }
            let linea = linea.map_err(|e| ErrorMotor::Conexion(e.to_string()))?;
            let Some(datos) = linea.strip_prefix("data:").map(str::trim_start) else {
                continue; // SSE empty lines and comments
            };
            if datos == "[DONE]" {
                break;
            }
            let trozo = interpretar(datos)?;
            if let Some((procesados, total)) = trozo.progreso {
                al_recibir(Evento::Leyendo { procesados, total });
            }
            if let Some(r) = &trozo.razonamiento {
                resumen.razonamiento.push_str(r);
                al_recibir(Evento::Razonamiento(r));
            }
            if let Some(t) = &trozo.texto {
                resumen.texto.push_str(t);
                al_recibir(Evento::Texto(t));
            }
            if let Some(fin) = trozo.fin {
                resumen.fin = fin;
            }
            if let Some(t) = trozo.tiempos {
                resumen.tokens_leidos = t.leidos;
                resumen.tokens_cache = t.cache;
                resumen.tokens_generados = t.generados;
                resumen.lectura_por_segundo = t.lectura;
                resumen.escritura_por_segundo = t.escritura;
            }
        }
        // Only numbers: the conversation text NEVER goes to the log (ADR 0014).
        log::info!(
            "respuesta: {} tokens leídos ({} de caché), {} generados a {:.1} tokens/s, fin {:?}",
            resumen.tokens_leidos,
            resumen.tokens_cache,
            resumen.tokens_generados,
            resumen.escritura_por_segundo.unwrap_or(0.0),
            resumen.fin
        );
        Ok(resumen)
    }

    /// Is the engine generating any answer right now?
    pub fn ocupado(&self) -> Result<bool, ErrorMotor> {
        let respuesta = self
            .agente
            .get(&format!("{}/slots", self.base_url()))
            .header("Authorization", &self.cabecera_clave())
            .call()
            .map_err(|e| ErrorMotor::Conexion(e.to_string()))?;
        let estado = respuesta.status().as_u16();
        let texto = respuesta
            .into_body()
            .read_to_string()
            .map_err(|e| ErrorMotor::Conexion(e.to_string()))?;
        if estado != 200 {
            return Err(error_de_respuesta(estado, &texto));
        }
        let huecos: Value =
            serde_json::from_str(&texto).map_err(|e| ErrorMotor::Protocolo(e.to_string()))?;
        let huecos = huecos
            .as_array()
            .ok_or_else(|| ErrorMotor::Protocolo("/slots no devolvió una lista".into()))?;
        Ok(huecos
            .iter()
            .any(|h| h["is_processing"].as_bool() == Some(true)))
    }
}

/// The JSON sent to the engine.
pub(crate) fn cuerpo(peticion: &PeticionChat) -> Value {
    let mensajes: Vec<Value> = peticion
        .mensajes
        .iter()
        .map(|m| json!({ "role": m.rol.como_texto(), "content": m.contenido }))
        .collect();
    let mut cuerpo = json!({
        "messages": mensajes,
        "stream": true,
        "return_progress": true, // notices while it reads the prompt (progress and cancellation points)
    });
    if let Some(n) = peticion.max_tokens {
        cuerpo["max_tokens"] = json!(n);
    }
    if let Some(t) = peticion.temperatura {
        cuerpo["temperature"] = json!(t);
    }
    if let Some(s) = peticion.semilla {
        cuerpo["seed"] = json!(s);
    }
    cuerpo
}

// ─── Interpreting the answer ──────────────────────────────────────────────────

/// What we care about in an SSE chunk (everything else is ignored).
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Trozo {
    pub progreso: Option<(u64, u64)>,
    pub razonamiento: Option<String>,
    pub texto: Option<String>,
    pub fin: Option<Fin>,
    pub tiempos: Option<Tiempos>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct Tiempos {
    pub leidos: u64,
    pub cache: u64,
    pub generados: u64,
    pub lectura: Option<f64>,
    pub escritura: Option<f64>,
}

/// Interprets the JSON of a `data:` line from the engine.
pub(crate) fn interpretar(datos: &str) -> Result<Trozo, ErrorMotor> {
    let valor: Value =
        serde_json::from_str(datos).map_err(|e| ErrorMotor::Protocolo(e.to_string()))?;
    if valor.get("error").is_some() {
        // An error in the middle of the answer.
        return Err(error_de_respuesta(500, datos));
    }
    let texto_no_vacio = |v: &Value| v.as_str().filter(|s| !s.is_empty()).map(str::to_owned);
    let opcion = &valor["choices"][0];
    let delta = &opcion["delta"];
    let progreso = valor.get("prompt_progress").map(|p| {
        let total = p["total"].as_u64().unwrap_or(0);
        let cache = p["cache"].as_u64().unwrap_or(0);
        let procesados = p["processed"].as_u64().unwrap_or(0);
        // The engine counts the cache inside "total"; we only show what really has to be read.
        (
            procesados.saturating_sub(cache),
            total.saturating_sub(cache),
        )
    });
    let fin = match opcion["finish_reason"].as_str() {
        Some("length") => Some(Fin::Limite),
        Some(_) => Some(Fin::Completo),
        None => None,
    };
    let tiempos = valor.get("timings").map(|t| Tiempos {
        leidos: t["prompt_n"].as_u64().unwrap_or(0),
        cache: t["cache_n"].as_u64().unwrap_or(0),
        generados: t["predicted_n"].as_u64().unwrap_or(0),
        lectura: t["prompt_per_second"].as_f64(),
        escritura: t["predicted_per_second"].as_f64(),
    });
    Ok(Trozo {
        progreso,
        razonamiento: texto_no_vacio(&delta["reasoning_content"]),
        texto: texto_no_vacio(&delta["content"]),
        fin,
        tiempos,
    })
}

/// Converts an engine error response into [`ErrorMotor::Respuesta`], keeping only the type and the message (part
/// 8c will translate them into understandable messages).
pub(crate) fn error_de_respuesta(estado: u16, texto: &str) -> ErrorMotor {
    let valor: Value = serde_json::from_str(texto).unwrap_or(Value::Null);
    let error = &valor["error"];
    let estado = error["code"]
        .as_u64()
        .and_then(|c| u16::try_from(c).ok())
        .unwrap_or(estado);
    ErrorMotor::Respuesta {
        estado,
        tipo: error["type"].as_str().unwrap_or("desconocido").to_owned(),
        mensaje: error["message"].as_str().unwrap_or("").to_owned(),
    }
}
