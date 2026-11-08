//! # locall-compat
//!
//! Responde a la pregunta clave de Locall: **¿cabe este modelo en tu equipo y lo entiende el motor?**
//!
//! - [`estimar_memoria`]: RAM que necesitará el modelo = tamaño de sus archivos + caché KV para el contexto pedido
//!   (ADR 0005: con carga sin mapeo, la memoria real es ~1,05–1,1 veces el archivo; la fórmula se comparó con lo
//!   medido en la Fase 0: 3,10 GB estimados frente a 2,95 GB medidos con el 4B y contexto de 4K).
//! - [`encaje`]: veredicto según la memoria **disponible de verdad** (no la que informa Vulkan, ADR 0007).
//! - [`comprobar`]: problemas que impiden cargar el modelo (arquitectura o cuantización que el motor fijado no
//!   conoce) y avisos que no lo impiden.
//!
//! Todo son funciones puras (sin acceso al disco ni al sistema), fáciles de probar.

mod arquitecturas;

pub use arquitecturas::ARQUITECTURAS_B12189;

use std::fmt;

use locall_gguf::Cabecera;

/// Memoria que siempre se deja libre para Windows y el resto de programas: 1,5 GB, la misma regla que usó el
/// vigilante de las pruebas de la Fase 0.
pub const RESERVA_SISTEMA: u64 = 1_500_000_000;

/// Si después de cargar el modelo quedan menos de 4 GB libres, el equipo va "justo": funciona, pero conviene
/// cerrar otros programas (con el 7B en la Fase 0 quedaron ~3,2 GB).
pub const UMBRAL_JUSTO: u64 = 4_000_000_000;

/// Número de tipos de tensor de ggml en la build 12189 (`GGML_TYPE_COUNT`).
const TIPOS_DE_TENSOR: u32 = 44;
/// Tipos de tensor que ggml eliminó del formato (`ggml.h`, build 12189): los GGUF que los usen ya no cargan.
const TIPOS_ELIMINADOS: [u32; 8] = [4, 5, 31, 32, 33, 36, 37, 38];

// ─── Memoria ──────────────────────────────────────────────────────────────────

/// Memoria estimada para usar un modelo, en bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Estimacion {
    /// Los pesos: el tamaño de los archivos del modelo.
    pub pesos: u64,
    /// La caché KV para el contexto pedido (`None` si la cabecera no trae los datos necesarios).
    pub cache_kv: Option<u64>,
    /// Lo que se usará en total.
    pub total: u64,
}

/// Estima la RAM que necesitará un modelo con un contexto de `contexto` tokens (caché KV en f16).
///
/// `bytes_archivos` es la suma del tamaño de todos sus archivos (en un modelo dividido, todas las partes).
pub fn estimar_memoria(cabecera: &Cabecera, bytes_archivos: u64, contexto: u64) -> Estimacion {
    let cache_kv = cabecera.cache_kv_bytes(contexto, 2);
    Estimacion {
        pesos: bytes_archivos,
        cache_kv,
        total: bytes_archivos.saturating_add(cache_kv.unwrap_or(0)),
    }
}

/// Qué tal cabe un modelo en la memoria disponible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Encaje {
    /// Ocupa como mucho una cuarta parte de lo disponible.
    Holgado,
    /// Cabe con margen.
    Cabe,
    /// Cabe, pero quedarán menos de 4 GB libres: conviene cerrar otros programas.
    Justo,
    /// No cabe respetando la reserva del sistema.
    NoCabe,
}

impl Encaje {
    /// Texto corto para la interfaz.
    pub fn descripcion(self) -> &'static str {
        match self {
            Encaje::Holgado => "Cabe con holgura",
            Encaje::Cabe => "Cabe",
            Encaje::Justo => "En el límite: cierra otros programas",
            Encaje::NoCabe => "No cabe en la memoria disponible",
        }
    }
}

/// Veredicto para un modelo que necesita `estimado` bytes cuando hay `disponible` bytes libres.
pub fn encaje(estimado: u64, disponible: u64) -> Encaje {
    let utilizable = disponible.saturating_sub(RESERVA_SISTEMA);
    if estimado > utilizable {
        Encaje::NoCabe
    } else if disponible - estimado < UMBRAL_JUSTO {
        Encaje::Justo
    } else if estimado <= disponible / 4 {
        Encaje::Holgado
    } else {
        Encaje::Cabe
    }
}

// ─── Compatibilidad con el motor ──────────────────────────────────────────────

/// Algo que impide usar el modelo con el motor fijado (build 12189).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problema {
    /// La cabecera no dice qué arquitectura es.
    SinArquitectura,
    /// El motor no conoce esta arquitectura.
    ArquitecturaNoSoportada(String),
    /// El modelo usa un tipo de tensor (cuantización) que el motor ya no o todavía no admite.
    TipoDeTensorNoSoportado(u32),
}

/// Algo que no impide usar el modelo, pero conviene saber.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Aviso {
    /// Sin plantilla de chat: el motor usará una genérica y las respuestas pueden salir mal formateadas.
    SinPlantillaChat,
    /// Se pidió más contexto del que admite el modelo; el motor lo recortará al máximo.
    ContextoRecortado { pedido: u64, maximo: u64 },
}

impl fmt::Display for Problema {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problema::SinArquitectura => write!(f, "el modelo no indica su arquitectura"),
            Problema::ArquitecturaNoSoportada(a) => {
                write!(f, "el motor no conoce la arquitectura «{a}»")
            }
            Problema::TipoDeTensorNoSoportado(t) => {
                write!(f, "el modelo usa un tipo de cuantización no admitido ({t})")
            }
        }
    }
}

impl fmt::Display for Aviso {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Aviso::SinPlantillaChat => write!(
                f,
                "el modelo no trae plantilla de chat; las respuestas podrían salir mal formateadas"
            ),
            Aviso::ContextoRecortado { pedido, maximo } => write!(
                f,
                "se pidieron {pedido} tokens de contexto pero el modelo admite {maximo}; se usará {maximo}"
            ),
        }
    }
}

/// Resultado de comprobar un modelo contra el motor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Comprobacion {
    pub problemas: Vec<Problema>,
    pub avisos: Vec<Aviso>,
}

impl Comprobacion {
    /// ¿Se puede usar el modelo?
    pub fn es_compatible(&self) -> bool {
        self.problemas.is_empty()
    }
}

/// Comprueba si el motor fijado puede usar el modelo con el contexto pedido.
pub fn comprobar(cabecera: &Cabecera, contexto: u64) -> Comprobacion {
    let mut c = Comprobacion::default();
    let resumen = cabecera.resumen();

    match resumen.arquitectura.as_deref() {
        None => c.problemas.push(Problema::SinArquitectura),
        Some(a) if !ARQUITECTURAS_B12189.contains(&a) => {
            c.problemas
                .push(Problema::ArquitecturaNoSoportada(a.to_string()));
        }
        Some(_) => {}
    }
    for &tipo in &cabecera.tipos_tensor {
        if tipo >= TIPOS_DE_TENSOR || TIPOS_ELIMINADOS.contains(&tipo) {
            c.problemas.push(Problema::TipoDeTensorNoSoportado(tipo));
        }
    }

    if !resumen.plantilla_chat {
        c.avisos.push(Aviso::SinPlantillaChat);
    }
    if let Some(maximo) = resumen.contexto_max
        && contexto > maximo
    {
        c.avisos.push(Aviso::ContextoRecortado {
            pedido: contexto,
            maximo,
        });
    }
    c
}

// ─── Todo junto ───────────────────────────────────────────────────────────────

/// La evaluación completa de un modelo para un equipo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evaluacion {
    pub estimacion: Estimacion,
    pub encaje: Encaje,
    pub comprobacion: Comprobacion,
}

/// Evalúa un modelo: memoria, encaje y compatibilidad.
pub fn evaluar(
    cabecera: &Cabecera,
    bytes_archivos: u64,
    contexto: u64,
    memoria_disponible: u64,
) -> Evaluacion {
    // Si se pide más contexto del que admite el modelo, se estima con el que se usará de verdad.
    let contexto_real = cabecera
        .resumen()
        .contexto_max
        .map_or(contexto, |m| contexto.min(m));
    let estimacion = estimar_memoria(cabecera, bytes_archivos, contexto_real);
    Evaluacion {
        encaje: encaje(estimacion.total, memoria_disponible),
        comprobacion: comprobar(cabecera, contexto),
        estimacion,
    }
}

// ─── Pruebas ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod pruebas {
    use std::collections::{BTreeMap, BTreeSet};

    use locall_gguf::Valor;

    use super::*;

    /// Una cabecera hecha a mano con los datos de Qwen3 4B (los de la Fase 0).
    fn cabecera_4b(arquitectura: &str, tipos: &[u32], plantilla: bool) -> Cabecera {
        let mut m = BTreeMap::new();
        let mut u = |k: &str, v: u32| {
            m.insert(format!("{arquitectura}.{k}"), Valor::U32(v));
        };
        u("block_count", 36);
        u("context_length", 40_960);
        u("attention.head_count", 32);
        u("attention.head_count_kv", 8);
        u("attention.key_length", 128);
        m.insert(
            "general.architecture".into(),
            Valor::Texto(arquitectura.into()),
        );
        if plantilla {
            m.insert("tokenizer.chat_template".into(), Valor::Texto("…".into()));
        }
        Cabecera {
            version: 3,
            tensores: tipos.len() as u64,
            metadatos: m,
            bytes_cabecera: 0,
            tipos_tensor: tipos.iter().copied().collect::<BTreeSet<_>>(),
        }
    }

    const GB: u64 = 1_000_000_000;

    #[test]
    fn estima_como_en_la_fase_0() {
        let c = cabecera_4b("qwen3", &[0, 12, 14], true);
        let e = estimar_memoria(&c, 2_497_280_256, 4_096);
        assert_eq!(e.cache_kv, Some(576 * 1024 * 1024));
        // 2,50 GB de archivo + 0,60 GB de caché = 3,10 GB (la Fase 0 midió 2,95 GB: la estimación es prudente).
        assert_eq!(e.total, 2_497_280_256 + 603_979_776);
    }

    #[test]
    fn da_el_veredicto_esperado_en_el_equipo_de_referencia() {
        // Con ~8,15 GB disponibles, como en la Fase 0:
        let disponible = 8_150_000_000;
        assert_eq!(encaje(1_300_000_000, disponible), Encaje::Holgado); // 1.5B
        assert_eq!(encaje(3_100_000_000, disponible), Encaje::Cabe); // 4B
        assert_eq!(encaje(4_700_000_000, disponible), Encaje::Justo); // 7B
        assert_eq!(encaje(7 * GB, disponible), Encaje::NoCabe);
        assert_eq!(encaje(1, 0), Encaje::NoCabe);
    }

    #[test]
    fn un_modelo_normal_es_compatible() {
        let c = cabecera_4b("qwen3", &[0, 12, 14], true);
        let r = comprobar(&c, 4_096);
        assert!(r.es_compatible());
        assert!(r.avisos.is_empty());
    }

    #[test]
    fn detecta_arquitecturas_y_cuantizaciones_desconocidas() {
        let c = cabecera_4b("arquitectura-inventada", &[0, 31, 99], true);
        let r = comprobar(&c, 4_096);
        assert!(!r.es_compatible());
        assert!(r.problemas.contains(&Problema::ArquitecturaNoSoportada(
            "arquitectura-inventada".into()
        )));
        assert!(r.problemas.contains(&Problema::TipoDeTensorNoSoportado(31))); // eliminado de ggml
        assert!(r.problemas.contains(&Problema::TipoDeTensorNoSoportado(99))); // no existe
    }

    #[test]
    fn avisa_sin_bloquear() {
        let c = cabecera_4b("qwen3", &[12], false);
        let r = comprobar(&c, 100_000);
        assert!(r.es_compatible());
        assert!(r.avisos.contains(&Aviso::SinPlantillaChat));
        assert!(r.avisos.contains(&Aviso::ContextoRecortado {
            pedido: 100_000,
            maximo: 40_960
        }));
    }

    #[test]
    fn la_lista_de_arquitecturas_incluye_las_probadas() {
        for a in ["qwen2", "qwen3", "llama", "gemma3", "smollm3"] {
            assert!(ARQUITECTURAS_B12189.contains(&a), "{a}");
        }
        assert!(!ARQUITECTURAS_B12189.contains(&"(unknown)"));
    }
}
