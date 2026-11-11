//! # locall-compat
//!
//! Answers Locall's key question: **does this model fit on your machine, and can the engine run it?**
//!
//! - [`estimar_memoria`]: RAM the model will need = the size of its files + KV cache for the requested context
//!   (ADR 0005: when loading without mmap, real memory is ~1.05–1.1 times the file; the formula was compared with the
//!   Phase 0 measurements: 3.10 GB estimated vs 2.95 GB measured for the 4B with a 4K context).
//! - [`encaje`]: verdict based on memory **actually available** (not what Vulkan reports, ADR 0007).
//! - [`comprobar`]: problems that prevent loading the model (an architecture or quantization the pinned engine does
//!   not know) and warnings that do not.
//!
//! These are all pure functions (no disk or system access), easy to test.

mod arquitecturas;

pub use arquitecturas::ARQUITECTURAS_B12189;

use std::fmt;

use locall_gguf::Cabecera;

/// Memory always left free for Windows and other programs: 1.5 GB, the same rule the watchdog used in the Phase 0
/// tests.
pub const RESERVA_SISTEMA: u64 = 1_500_000_000;

/// If less than 4 GB is left free after loading the model, the machine is "tight": it works, but closing other
/// programs is advisable (with the 7B in Phase 0, ~3.2 GB were left).
pub const UMBRAL_JUSTO: u64 = 4_000_000_000;

/// Number of ggml tensor types in build 12189 (`GGML_TYPE_COUNT`).
const TIPOS_DE_TENSOR: u32 = 44;
/// Tensor types that ggml removed from the format (`ggml.h`, build 12189): GGUFs that use them no longer load.
const TIPOS_ELIMINADOS: [u32; 8] = [4, 5, 31, 32, 33, 36, 37, 38];

// ─── Memory ───────────────────────────────────────────────────────────────────

/// Estimated memory needed to use a model, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Estimacion {
    /// The weights: the size of the model's files.
    pub pesos: u64,
    /// The KV cache for the requested context (`None` if the header lacks the necessary data).
    pub cache_kv: Option<u64>,
    /// What will be used in total.
    pub total: u64,
}

/// Estimates the RAM a model will need with a context of `contexto` tokens (KV cache in f16).
///
/// `bytes_archivos` is the sum of the size of all its files (for a split model, every part).
pub fn estimar_memoria(cabecera: &Cabecera, bytes_archivos: u64, contexto: u64) -> Estimacion {
    let cache_kv = cabecera.cache_kv_bytes(contexto, 2);
    Estimacion {
        pesos: bytes_archivos,
        cache_kv,
        total: bytes_archivos.saturating_add(cache_kv.unwrap_or(0)),
    }
}

/// How well a model fits in the available memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Encaje {
    /// Takes at most a quarter of what is available.
    Holgado,
    /// Fits with room to spare.
    Cabe,
    /// Fits, but less than 4 GB will be left free: closing other programs is advisable.
    Justo,
    /// Does not fit while respecting the system reserve.
    NoCabe,
}

impl Encaje {
    /// Short text for the UI.
    pub fn descripcion(self) -> &'static str {
        match self {
            Encaje::Holgado => "Cabe con holgura",
            Encaje::Cabe => "Cabe",
            Encaje::Justo => "En el límite: cierra otros programas",
            Encaje::NoCabe => "No cabe en la memoria disponible",
        }
    }
}

/// Verdict for a model that needs `estimado` bytes when `disponible` bytes are free.
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

// ─── Engine compatibility ─────────────────────────────────────────────────────

/// Something that prevents using the model with the pinned engine (build 12189).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problema {
    /// The header does not say which architecture it is.
    SinArquitectura,
    /// The engine does not know this architecture.
    ArquitecturaNoSoportada(String),
    /// The model uses a tensor type (quantization) that the engine no longer supports or does not support yet.
    TipoDeTensorNoSoportado(u32),
}

/// Something that does not prevent using the model but is worth knowing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Aviso {
    /// No chat template: the engine will use a generic one and answers may come out badly formatted.
    SinPlantillaChat,
    /// More context than the model supports was requested; the engine will cap it at the maximum.
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

/// Result of checking a model against the engine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Comprobacion {
    pub problemas: Vec<Problema>,
    pub avisos: Vec<Aviso>,
}

impl Comprobacion {
    /// Can the model be used?
    pub fn es_compatible(&self) -> bool {
        self.problemas.is_empty()
    }
}

/// Checks whether the pinned engine can use the model with the requested context.
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

// ─── All together ─────────────────────────────────────────────────────────────

/// The complete evaluation of a model for a machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evaluacion {
    pub estimacion: Estimacion,
    pub encaje: Encaje,
    pub comprobacion: Comprobacion,
}

/// Evaluates a model: memory, fit and compatibility.
pub fn evaluar(
    cabecera: &Cabecera,
    bytes_archivos: u64,
    contexto: u64,
    memoria_disponible: u64,
) -> Evaluacion {
    // If more context than the model supports is requested, the estimate uses the one that will really be used.
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

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod pruebas {
    use std::collections::{BTreeMap, BTreeSet};

    use locall_gguf::Valor;

    use super::*;

    /// A hand-made header with Qwen3 4B's data (the Phase 0 one).
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
        // 2.50 GB of file + 0.60 GB of cache = 3.10 GB (Phase 0 measured 2.95 GB: the estimate is conservative).
        assert_eq!(e.total, 2_497_280_256 + 603_979_776);
    }

    #[test]
    fn da_el_veredicto_esperado_en_el_equipo_de_referencia() {
        // With ~8.15 GB available, as in Phase 0:
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
        assert!(r.problemas.contains(&Problema::TipoDeTensorNoSoportado(31))); // removed from ggml
        assert!(r.problemas.contains(&Problema::TipoDeTensorNoSoportado(99))); // does not exist
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
