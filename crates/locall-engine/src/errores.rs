//! Errores comprensibles (decisión D8 de la Fase 0).
//!
//! `llama-server` explica sus fallos en inglés, a su manera y mezclados con datos privados (la ruta del modelo
//! lleva el nombre de usuario de Windows). Aquí se **reconocen** los fallos más comunes y se convierten en una
//! [`Causa`] con un mensaje propio en español, que nunca copia el texto del motor.
//!
//! Los textos que se buscan se han comprobado provocando cada fallo con la build b12189. Si una versión futura del
//! motor cambia sus mensajes, el fallo se clasifica como [`Causa::Desconocida`] y se remite al registro: el error se
//! sigue viendo, solo que con un mensaje menos concreto.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// Cuánto del final del registro se lee para buscar la causa (los fallos de carga están al final).
const FINAL_DEL_REGISTRO: u64 = 64 * 1024;

/// Qué ha pasado, en términos que entiende el usuario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Causa {
    /// Otro programa ocupó el puerto elegido justo antes de que el motor lo abriera.
    PuertoOcupado,
    /// No hay memoria para el modelo o para su contexto.
    MemoriaInsuficiente,
    /// El archivo del modelo no es válido, está dañado o incompleto.
    ModeloDanado,
    /// Esta versión del motor no sabe ejecutar ese tipo de modelo.
    ArquitecturaNoSoportada(String),
    /// La conversación no cabe en el contexto.
    ContextoExcedido {
        pedidos: Option<u64>,
        maximo: Option<u64>,
    },
    /// El modelo todavía se está cargando.
    Cargando,
    /// El motor rechazó la clave interna: es un fallo de Locall, no del usuario.
    ClaveRechazada,
    /// No se ha reconocido.
    Desconocida,
}

impl Causa {
    /// Mensaje para el usuario, o `None` si la causa es desconocida.
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

/// Busca la causa de un cierre durante la carga en el texto del registro del motor.
pub(crate) fn causa_en_registro(registro: &str) -> Causa {
    // Primero los mensajes más concretos: "failed to load model" aparece también en los demás fallos de carga.
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
        // El cargador no reconoce el archivo (por ejemplo, no empieza por "GGUF").
        return Causa::ModeloDanado;
    }
    Causa::Desconocida
}

/// Lee el final del registro y busca la causa. Si no se puede leer, la causa es desconocida.
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

/// Causa de una respuesta de error del motor (por su tipo, que es más estable que el mensaje).
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

/// Los números que empiezan justo tras un `(`: en "(6030 tokens) … (2048 tokens)" → 6030, 2048.
fn numeros_entre_parentesis(texto: &str) -> impl Iterator<Item = u64> + '_ {
    texto.split('(').skip(1).filter_map(|trozo| {
        let digitos: String = trozo.chars().take_while(char::is_ascii_digit).collect();
        digitos.parse().ok()
    })
}
