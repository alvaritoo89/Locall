//! Understandable errors (Phase 0 decision D8).
//!
//! `llama-server` explains its failures in English, its own way and mixed with private data (the model path carries
//! the Windows user name). Here the most common failures are **recognized** and turned into a [`Causa`] with our own
//! message in Spanish, which never copies the engine's text.
//!
//! The texts searched for were checked by triggering each failure with build b12189. If a future engine version
//! changes its messages, the failure is classified as [`Causa::Desconocida`] and points to the log: the error is
//! still visible, just with a less specific message.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// How much of the end of the log is read to look for the cause (load failures are at the end).
const FINAL_DEL_REGISTRO: u64 = 64 * 1024;

/// What happened, in terms the user understands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Causa {
    /// Another program took the chosen port just before the engine opened it.
    PuertoOcupado,
    /// There is no memory for the model or for its context.
    MemoriaInsuficiente,
    /// The model file is not valid, is damaged or incomplete.
    ModeloDanado,
    /// This engine version cannot run that kind of model.
    ArquitecturaNoSoportada(String),
    /// The conversation does not fit in the context.
    ContextoExcedido {
        pedidos: Option<u64>,
        maximo: Option<u64>,
    },
    /// The model is still loading.
    Cargando,
    /// The engine rejected the internal key: it is a Locall bug, not the user's fault.
    ClaveRechazada,
    /// Not recognized.
    Desconocida,
}

impl Causa {
    /// Message for the user, or `None` if the cause is unknown.
    pub fn mensaje(&self) -> Option<String> {
        let texto = match self {
            Causa::PuertoOcupado => {
                "otro programa ocupó el puerto local que iba a usar el motor; vuelve a intentarlo"
                    .into()
            }
            Causa::MemoriaInsuficiente => {
                "no hay memoria suficiente para este modelo con este tamaño de \
                 contexto; prueba con un contexto más pequeño o cierra otros programas"
                    .into()
            }
            Causa::ModeloDanado => {
                "el archivo del modelo no es válido o está incompleto; vuelve a descargarlo".into()
            }
            Causa::ArquitecturaNoSoportada(nombre) => {
                format!("esta versión del motor no admite modelos de tipo «{nombre}»")
            }
            Causa::ContextoExcedido {
                pedidos: Some(p),
                maximo: Some(m),
            } => format!(
                "la conversación ({p} tokens) no cabe en el contexto del modelo ({m} tokens); empieza una \
                 conversación nueva o aumenta el contexto"
            ),
            Causa::ContextoExcedido { .. } => {
                "la conversación no cabe en el contexto del modelo; empieza \
                 una conversación nueva o aumenta el contexto"
                    .into()
            }
            Causa::Cargando => "el modelo todavía se está cargando".into(),
            Causa::ClaveRechazada => {
                "el motor rechazó la clave interna de Locall (fallo interno; por favor, avísanos)"
                    .into()
            }
            Causa::Desconocida => return None,
        };
        Some(texto)
    }
}

/// Looks for the cause of an exit during loading in the engine's log text.
pub(crate) fn causa_en_registro(registro: &str) -> Causa {
    // The most specific messages first: "failed to load model" also appears in the other load failures.
    if registro.contains("couldn't bind HTTP server socket") {
        return Causa::PuertoOcupado;
    }
    if let Some(resto) = registro.split("unknown model architecture: '").nth(1) {
        let nombre: String = resto
            .chars()
            .take_while(|&c| c != '\'' && c != '\n')
            .take(64)
            .collect();
        return Causa::ArquitecturaNoSoportada(nombre);
    }
    if registro.contains("not within the file bounds")
        || registro.contains("corrupted or incomplete")
    {
        return Causa::ModeloDanado;
    }
    if registro.contains("failed to allocate") || registro.contains("out of memory") {
        return Causa::MemoriaInsuficiente;
    }
    if registro.contains("llama_model_loader: failed to load model from") {
        // The loader does not recognize the file (for example, it does not start with "GGUF").
        return Causa::ModeloDanado;
    }
    Causa::Desconocida
}

/// Reads the end of the log and looks for the cause. If it cannot be read, the cause is unknown.
pub(crate) fn causa_en_archivo(ruta: &Path) -> Causa {
    leer_final(ruta).map_or(Causa::Desconocida, |texto| causa_en_registro(&texto))
}

fn leer_final(ruta: &Path) -> io::Result<String> {
    let mut archivo = File::open(ruta)?;
    let tamano = archivo.metadata()?.len();
    archivo.seek(SeekFrom::Start(tamano.saturating_sub(FINAL_DEL_REGISTRO)))?;
    let mut bytes = Vec::new();
    archivo.read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Cause of an engine error response (by its type, which is more stable than the message).
pub(crate) fn causa_de_respuesta(estado: u16, tipo: &str, mensaje: &str) -> Causa {
    match tipo {
        "exceed_context_size_error" => {
            // "request (6030 tokens) exceeds the available context size (2048 tokens), …"
            let mut numeros = numeros_entre_parentesis(mensaje);
            Causa::ContextoExcedido {
                pedidos: numeros.next(),
                maximo: numeros.next(),
            }
        }
        "authentication_error" => Causa::ClaveRechazada,
        "unavailable_error" => Causa::Cargando,
        _ if estado == 401 => Causa::ClaveRechazada,
        _ if estado == 503 => Causa::Cargando,
        _ => Causa::Desconocida,
    }
}

/// The numbers that start right after a `(`: in "(6030 tokens) … (2048 tokens)" → 6030, 2048.
fn numeros_entre_parentesis(texto: &str) -> impl Iterator<Item = u64> + '_ {
    texto.split('(').skip(1).filter_map(|trozo| {
        let digitos: String = trozo.chars().take_while(char::is_ascii_digit).collect();
        digitos.parse().ok()
    })
}
