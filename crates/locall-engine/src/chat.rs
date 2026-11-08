//! Conversación con el motor: petición de chat con respuesta por trozos (SSE), cancelación y traducción.
//!
//! **Traducción (D4):** las respuestas de `llama-server` incluyen datos que no deben salir de aquí (por ejemplo, la
//! ruta completa del modelo en el campo `model`). Por eso nunca se reenvían tal cual: se leen solo los campos que
//! necesitamos y se devuelven con tipos propios ([`Evento`], [`Resumen`]).

use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};

use crate::{ErrorMotor, Motor};

// ─── Tipos públicos ───────────────────────────────────────────────────────────

/// Quién dice un mensaje.
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

/// Un mensaje de la conversación.
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

/// Una petición de chat.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PeticionChat {
    pub mensajes: Vec<Mensaje>,
    /// Máximo de tokens a generar (`None` = hasta que el modelo termine o se llene el contexto).
    pub max_tokens: Option<u32>,
    /// Creatividad (0 = siempre lo más probable). `None` = la del modelo.
    pub temperatura: Option<f32>,
    /// Semilla aleatoria, para respuestas repetibles.
    pub semilla: Option<u32>,
}

/// Lo que va llegando mientras el motor trabaja.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Evento<'a> {
    /// Leyendo el prompt: `procesados` de `total` tokens.
    Leyendo { procesados: u64, total: u64 },
    /// Un trozo del razonamiento (modelos que "piensan", como Qwen3).
    Razonamiento(&'a str),
    /// Un trozo de la respuesta.
    Texto(&'a str),
}

/// Por qué terminó la respuesta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fin {
    /// El modelo terminó por sí mismo.
    Completo,
    /// Se alcanzó el máximo de tokens.
    Limite,
    /// Se canceló a petición.
    Cancelado,
}

/// Resultado de una respuesta completa (o cancelada: con lo que dio tiempo a generar).
#[derive(Debug, Clone, PartialEq)]
pub struct Resumen {
    pub texto: String,
    pub razonamiento: String,
    pub fin: Fin,
    /// Tokens del prompt que se han leído (sin contar los reutilizados de la caché).
    pub tokens_leidos: u64,
    /// Tokens del prompt reutilizados de una petición anterior.
    pub tokens_cache: u64,
    pub tokens_generados: u64,
    /// Velocidad de lectura del prompt (tokens/s).
    pub lectura_por_segundo: Option<f64>,
    /// Velocidad de escritura (tokens/s).
    pub escritura_por_segundo: Option<f64>,
}

// ─── Petición ─────────────────────────────────────────────────────────────────

impl Motor {
    /// Pide una respuesta y la entrega por trozos a `al_recibir` mientras se genera.
    ///
    /// Si `cancelar` se pone a `true`, se corta la conexión: `llama-server` deja de generar (comprobado en la
    /// Fase 0) y se devuelve lo generado hasta entonces con [`Fin::Cancelado`]. La cancelación se atiende cada vez
    /// que llega algo del motor: un token, o un aviso de progreso mientras lee el prompt (uno por lote).
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
                // Al salir, se suelta el lector y se cierra la conexión: el motor deja de generar.
                resumen.fin = Fin::Cancelado;
                log::info!("respuesta cancelada a petición");
                return Ok(resumen);
            }
            let linea = linea.map_err(|e| ErrorMotor::Conexion(e.to_string()))?;
            let Some(datos) = linea.strip_prefix("data:").map(str::trim_start) else {
                continue; // líneas vacías y comentarios de SSE
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
        // Solo cifras: el texto de la conversación NUNCA va al registro (ADR 0014).
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

    /// ¿Está el motor generando alguna respuesta ahora mismo?
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

/// El JSON que se envía al motor.
pub(crate) fn cuerpo(peticion: &PeticionChat) -> Value {
    let mensajes: Vec<Value> = peticion
        .mensajes
        .iter()
        .map(|m| json!({ "role": m.rol.como_texto(), "content": m.contenido }))
        .collect();
    let mut cuerpo = json!({
        "messages": mensajes,
        "stream": true,
        "return_progress": true, // avisos mientras lee el prompt (progreso y puntos de cancelación)
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

// ─── Interpretación de la respuesta ──────────────────────────────────────────

/// Lo que nos interesa de un trozo SSE (todo lo demás se ignora).
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

/// Interpreta el JSON de una línea `data:` del motor.
pub(crate) fn interpretar(datos: &str) -> Result<Trozo, ErrorMotor> {
    let valor: Value =
        serde_json::from_str(datos).map_err(|e| ErrorMotor::Protocolo(e.to_string()))?;
    if valor.get("error").is_some() {
        // Un error a mitad de la respuesta.
        return Err(error_de_respuesta(500, datos));
    }
    let texto_no_vacio = |v: &Value| v.as_str().filter(|s| !s.is_empty()).map(str::to_owned);
    let opcion = &valor["choices"][0];
    let delta = &opcion["delta"];
    let progreso = valor.get("prompt_progress").map(|p| {
        let total = p["total"].as_u64().unwrap_or(0);
        let cache = p["cache"].as_u64().unwrap_or(0);
        let procesados = p["processed"].as_u64().unwrap_or(0);
        // El motor cuenta la caché dentro de "total"; mostramos solo lo que hay que leer de verdad.
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

/// Convierte una respuesta de error del motor en [`ErrorMotor::Respuesta`], quedándose solo con el tipo y el
/// mensaje (la parte 8c los traducirá a mensajes comprensibles).
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
