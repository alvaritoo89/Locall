//! # locall-gguf
//!
//! Reader for **GGUF headers** (version 3), the format of llama.cpp models.
//!
//! Reads the model's "label" (metadata and tensor descriptions) **without loading the weights**, in order to:
//! - know the architecture, number of layers, maximum context, etc.;
//! - check compatibility before loading a model;
//! - estimate the KV cache memory (see ADR 0005).
//!
//! Format (per llama.cpp's `ggml/include/gguf.h`), all *little-endian*:
//! 1. Magic `GGUF` (4 bytes) and version (`u32`).
//! 2. Number of tensors and of key-value pairs (`i64`).
//! 3. The key-value pairs (metadata).
//! 4. The description of each tensor (name, dimensions, type, offset).
//!
//! ## Security
//!
//! A downloaded GGUF file is **untrusted input**: it could be damaged or tampered with. That is why every length
//! read from the file is checked against a **limit** before allocating memory, and errors are returned as
//! [`ErrorGguf`] instead of crashing the program.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;

/// Magic every GGUF file starts with.
pub const MAGIA: [u8; 4] = *b"GGUF";
/// The only format version we accept (ADR 0004).
pub const VERSION_SOPORTADA: u32 = 3;

// ─── Safety limits ────────────────────────────────────────────────────────────
// Generous for real models (a Qwen header takes ~6 MB), but they stop a malicious file from making us allocate
// gigabytes of memory or walk endless lists.

/// Maximum length of a text (keys, chat templates…).
const MAX_TEXTO: u64 = 64 * 1024 * 1024;
/// Maximum number of key-value pairs.
const MAX_METADATOS: u64 = 100_000;
/// Maximum number of tensors.
const MAX_TENSORES: u64 = 1_000_000;
/// Maximum number of dimensions of a tensor (llama.cpp uses 4).
const MAX_DIMENSIONES: u64 = 8;
/// Maximum number of elements in a list (the vocabulary is around 150,000).
const MAX_ELEMENTOS_LISTA: u64 = 100_000_000;
/// Maximum depth of lists inside lists.
const MAX_PROFUNDIDAD: u32 = 4;
/// What is allocated up front for a text (it then grows as the data arrives).
const RESERVA_INICIAL_TEXTO: u64 = 64 * 1024;

// ─── Public types ─────────────────────────────────────────────────────────────

/// A metadata value.
///
/// Lists do not keep their content (the vocabulary takes several MB): only their type and length.
#[derive(Debug, Clone, PartialEq)]
pub enum Valor {
    U8(u8),
    I8(i8),
    U16(u16),
    I16(i16),
    U32(u32),
    I32(i32),
    F32(f32),
    Bool(bool),
    Texto(String),
    Lista { tipo: u32, longitud: u64 },
    U64(u64),
    I64(i64),
    F64(f64),
}

impl Valor {
    /// Returns the value as an unsigned integer, if it is one (and not negative).
    pub fn como_u64(&self) -> Option<u64> {
        match *self {
            Valor::U8(v) => Some(u64::from(v)),
            Valor::U16(v) => Some(u64::from(v)),
            Valor::U32(v) => Some(u64::from(v)),
            Valor::U64(v) => Some(v),
            Valor::I8(v) => u64::try_from(v).ok(),
            Valor::I16(v) => u64::try_from(v).ok(),
            Valor::I32(v) => u64::try_from(v).ok(),
            Valor::I64(v) => u64::try_from(v).ok(),
            _ => None,
        }
    }

    /// Returns the value as text, if it is text.
    pub fn como_texto(&self) -> Option<&str> {
        match self {
            Valor::Texto(t) => Some(t),
            _ => None,
        }
    }
}

/// What can go wrong when reading a header.
#[derive(Debug)]
pub enum ErrorGguf {
    /// System read error (file not found, no permissions…).
    Io(io::Error),
    /// The file does not start with `GGUF`.
    NoEsGguf,
    /// Format version other than 3.
    VersionNoSoportada(u32),
    /// The file ends earlier than its own header says.
    Truncado,
    /// A value type that does not exist in the format.
    TipoDesconocido(u32),
    /// A text that is not valid UTF-8.
    TextoNoUtf8,
    /// A quantity read from the file exceeds our safety limits.
    FueraDeLimite {
        que: &'static str,
        valor: u64,
        maximo: u64,
    },
    /// Lists nested beyond what is reasonable.
    ListaDemasiadoProfunda,
}

impl fmt::Display for ErrorGguf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorGguf::Io(e) => write!(f, "error de lectura: {e}"),
            ErrorGguf::NoEsGguf => write!(f, "el archivo no es un GGUF (no empieza por \"GGUF\")"),
            ErrorGguf::VersionNoSoportada(v) => {
                write!(
                    f,
                    "versión de GGUF {v} no soportada (solo la {VERSION_SOPORTADA})"
                )
            }
            ErrorGguf::Truncado => write!(
                f,
                "el archivo está incompleto o dañado (se acaba antes de tiempo)"
            ),
            ErrorGguf::TipoDesconocido(t) => write!(f, "tipo de valor desconocido: {t}"),
            ErrorGguf::TextoNoUtf8 => write!(f, "hay un texto que no es UTF-8 válido"),
            ErrorGguf::FueraDeLimite { que, valor, maximo } => {
                write!(f, "{que}: {valor} supera el límite de seguridad ({maximo})")
            }
            ErrorGguf::ListaDemasiadoProfunda => write!(f, "listas anidadas demasiado profundas"),
        }
    }
}

impl std::error::Error for ErrorGguf {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ErrorGguf::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for ErrorGguf {
    fn from(e: io::Error) -> Self {
        // "Unexpected end of file" means, for us, a truncated file.
        if e.kind() == io::ErrorKind::UnexpectedEof {
            ErrorGguf::Truncado
        } else {
            ErrorGguf::Io(e)
        }
    }
}

/// The header of a GGUF model.
#[derive(Debug, Clone)]
pub struct Cabecera {
    pub version: u32,
    pub tensores: u64,
    pub metadatos: BTreeMap<String, Valor>,
    /// Bytes taken by the whole header (metadata + tensor descriptions).
    pub bytes_cabecera: u64,
    /// Data types (quantization) used by the tensors, by their ggml number (`ggml_type`).
    /// Used to check that the engine can read them.
    pub tipos_tensor: BTreeSet<u32>,
}

/// The header data Locall cares about most.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resumen {
    pub arquitectura: Option<String>,
    pub capas: Option<u64>,
    pub contexto_max: Option<u64>,
    pub cabezas: Option<u64>,
    pub cabezas_kv: Option<u64>,
    pub dim_clave: Option<u64>,
    pub dim_valor: Option<u64>,
    pub vocabulario: Option<u64>,
    pub plantilla_chat: bool,
    /// Number of files if the model is split (`split.count`).
    pub partes: Option<u64>,
}

impl Cabecera {
    /// The model architecture (`general.architecture`), for example `"qwen2"`.
    pub fn arquitectura(&self) -> Option<&str> {
        self.metadatos.get("general.architecture")?.como_texto()
    }

    /// Reads a number from the architecture's own keys: `"<architecture>.<suffix>"`.
    fn de_arquitectura(&self, sufijo: &str) -> Option<u64> {
        let arquitectura = self.arquitectura()?;
        self.metadatos
            .get(&format!("{arquitectura}.{sufijo}"))?
            .como_u64()
    }

    /// Gathers the main data into a [`Resumen`].
    pub fn resumen(&self) -> Resumen {
        let cabezas = self.de_arquitectura("attention.head_count");
        let embebido = self.de_arquitectura("embedding_length");
        // If the model does not state the dimension of each head, it is computed: embedding / heads.
        let dim_calculada = match (embebido, cabezas) {
            (Some(e), Some(c)) if c > 0 => Some(e / c),
            _ => None,
        };
        let dim_clave = self
            .de_arquitectura("attention.key_length")
            .or(dim_calculada);
        let vocabulario = match self.metadatos.get("tokenizer.ggml.tokens") {
            Some(Valor::Lista { longitud, .. }) => Some(*longitud),
            _ => None,
        };
        Resumen {
            arquitectura: self.arquitectura().map(str::to_owned),
            capas: self.de_arquitectura("block_count"),
            contexto_max: self.de_arquitectura("context_length"),
            cabezas,
            cabezas_kv: self.de_arquitectura("attention.head_count_kv"),
            dim_clave,
            dim_valor: self.de_arquitectura("attention.value_length").or(dim_clave),
            vocabulario,
            plantilla_chat: self.metadatos.contains_key("tokenizer.chat_template"),
            partes: self.metadatos.get("split.count").and_then(Valor::como_u64),
        }
    }

    /// Estimates the KV cache bytes for `contexto` tokens.
    ///
    /// Formula: context × layers × kv_heads × (key_dim + value_dim) × bytes_per_element.
    /// With the cache in `f16`, `bytes_per_element` is 2. Returns `None` if data is missing or on overflow.
    /// It is an approximation: it does not account for sliding-window attention (SWA) or recurrent models.
    pub fn cache_kv_bytes(&self, contexto: u64, bytes_por_elemento: u64) -> Option<u64> {
        let r = self.resumen();
        let capas = r.capas?;
        let cabezas_kv = r.cabezas_kv.or(r.cabezas)?;
        let por_cabeza = r.dim_clave?.checked_add(r.dim_valor?)?;
        contexto
            .checked_mul(capas)?
            .checked_mul(cabezas_kv)?
            .checked_mul(por_cabeza)?
            .checked_mul(bytes_por_elemento)
    }
}

// ─── Reading ──────────────────────────────────────────────────────────────────

/// Reads a GGUF header from any byte source (file, memory, network…).
pub fn leer<R: Read>(fuente: R) -> Result<Cabecera, ErrorGguf> {
    let mut l = Lector {
        interno: fuente,
        leidos: 0,
    };

    if l.bytes::<4>()? != MAGIA {
        return Err(ErrorGguf::NoEsGguf);
    }
    let version = l.u32()?;
    if version != VERSION_SOPORTADA {
        return Err(ErrorGguf::VersionNoSoportada(version));
    }
    let tensores = l.u64()?;
    limite("número de tensores", tensores, MAX_TENSORES)?;
    let n_metadatos = l.u64()?;
    limite("número de metadatos", n_metadatos, MAX_METADATOS)?;

    let mut metadatos = BTreeMap::new();
    for _ in 0..n_metadatos {
        let clave = l.texto()?;
        let tipo = l.u32()?;
        let valor = l.valor(tipo, 0)?;
        metadatos.insert(clave, valor);
    }

    // Of the tensor descriptions we only keep their type; the whole list has to be walked to know where the header
    // ends and to check that the file is complete.
    let mut tipos_tensor = BTreeSet::new();
    for _ in 0..tensores {
        l.saltar_texto()?; // name
        let dimensiones = u64::from(l.u32()?);
        limite("dimensiones de un tensor", dimensiones, MAX_DIMENSIONES)?;
        l.saltar(8 * dimensiones)?; // size of each dimension (i64)
        tipos_tensor.insert(l.u32()?);
        let _posicion = l.u64()?;
    }

    Ok(Cabecera {
        version,
        tensores,
        metadatos,
        bytes_cabecera: l.leidos,
        tipos_tensor,
    })
}

/// Opens a file and reads its header.
pub fn leer_archivo(ruta: impl AsRef<Path>) -> Result<Cabecera, ErrorGguf> {
    let archivo = File::open(ruta)?;
    leer(BufReader::new(archivo))
}

/// Returns an error if `valor` exceeds `maximo`.
fn limite(que: &'static str, valor: u64, maximo: u64) -> Result<(), ErrorGguf> {
    if valor > maximo {
        Err(ErrorGguf::FueraDeLimite { que, valor, maximo })
    } else {
        Ok(())
    }
}

/// Bytes taken by a fixed-size element (`None` for texts and lists, which have variable length).
fn tamano_fijo(tipo: u32) -> Option<u64> {
    match tipo {
        0 | 1 | 7 => Some(1), // u8, i8, bool
        2 | 3 => Some(2),     // u16, i16
        4..=6 => Some(4),     // u32, i32, f32
        10..=12 => Some(8),   // u64, i64, f64
        _ => None,
    }
}

/// Wrapper that reads from the source and counts how many bytes it has read.
struct Lector<R> {
    interno: R,
    leidos: u64,
}

impl<R: Read> Lector<R> {
    /// Reads exactly `N` bytes. `const N: usize` is a parameter fixed at compile time.
    fn bytes<const N: usize>(&mut self) -> Result<[u8; N], ErrorGguf> {
        let mut buffer = [0u8; N];
        self.interno.read_exact(&mut buffer)?;
        self.leidos += N as u64;
        Ok(buffer)
    }

    fn u8(&mut self) -> Result<u8, ErrorGguf> {
        Ok(self.bytes::<1>()?[0])
    }
    fn i8(&mut self) -> Result<i8, ErrorGguf> {
        Ok(i8::from_le_bytes(self.bytes::<1>()?))
    }
    fn u16(&mut self) -> Result<u16, ErrorGguf> {
        Ok(u16::from_le_bytes(self.bytes::<2>()?))
    }
    fn i16(&mut self) -> Result<i16, ErrorGguf> {
        Ok(i16::from_le_bytes(self.bytes::<2>()?))
    }
    fn u32(&mut self) -> Result<u32, ErrorGguf> {
        Ok(u32::from_le_bytes(self.bytes::<4>()?))
    }
    fn i32(&mut self) -> Result<i32, ErrorGguf> {
        Ok(i32::from_le_bytes(self.bytes::<4>()?))
    }
    fn f32(&mut self) -> Result<f32, ErrorGguf> {
        Ok(f32::from_le_bytes(self.bytes::<4>()?))
    }
    fn u64(&mut self) -> Result<u64, ErrorGguf> {
        Ok(u64::from_le_bytes(self.bytes::<8>()?))
    }
    fn i64(&mut self) -> Result<i64, ErrorGguf> {
        Ok(i64::from_le_bytes(self.bytes::<8>()?))
    }
    fn f64(&mut self) -> Result<f64, ErrorGguf> {
        Ok(f64::from_le_bytes(self.bytes::<8>()?))
    }

    /// Skips `n` bytes without keeping them (without allocating memory).
    fn saltar(&mut self, n: u64) -> Result<(), ErrorGguf> {
        let copiados = io::copy(&mut (&mut self.interno).take(n), &mut io::sink())?;
        if copiados < n {
            return Err(ErrorGguf::Truncado);
        }
        self.leidos += n;
        Ok(())
    }

    /// Reads a text: length (`u64`) + UTF-8 bytes. The length is limited, and memory is allocated **as the data
    /// arrives**, not according to the length the file claims: a 100-byte file saying «a 64 MB text comes next» does
    /// not make us allocate 64 MB (found by the malformed-input tests, ADR 0026).
    fn texto(&mut self) -> Result<String, ErrorGguf> {
        let longitud = self.u64()?;
        limite("longitud de un texto", longitud, MAX_TEXTO)?;
        let mut bytes = Vec::with_capacity(longitud.min(RESERVA_INICIAL_TEXTO) as usize);
        let llegados = (&mut self.interno).take(longitud).read_to_end(&mut bytes)?;
        if (llegados as u64) < longitud {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
        }
        self.leidos += longitud;
        String::from_utf8(bytes).map_err(|_| ErrorGguf::TextoNoUtf8)
    }

    /// Skips a text without keeping it (for tensor names and texts inside lists).
    fn saltar_texto(&mut self) -> Result<(), ErrorGguf> {
        let longitud = self.u64()?;
        limite("longitud de un texto", longitud, MAX_TEXTO)?;
        self.saltar(longitud)
    }

    /// Reads a value of the given type.
    fn valor(&mut self, tipo: u32, profundidad: u32) -> Result<Valor, ErrorGguf> {
        Ok(match tipo {
            0 => Valor::U8(self.u8()?),
            1 => Valor::I8(self.i8()?),
            2 => Valor::U16(self.u16()?),
            3 => Valor::I16(self.i16()?),
            4 => Valor::U32(self.u32()?),
            5 => Valor::I32(self.i32()?),
            6 => Valor::F32(self.f32()?),
            7 => Valor::Bool(self.u8()? != 0),
            8 => Valor::Texto(self.texto()?),
            9 => self.lista(profundidad)?,
            10 => Valor::U64(self.u64()?),
            11 => Valor::I64(self.i64()?),
            12 => Valor::F64(self.f64()?),
            otro => return Err(ErrorGguf::TipoDesconocido(otro)),
        })
    }

    /// Reads a list: element type (`u32`) + number of elements (`u64`) + elements.
    /// The elements are skipped; only the type and length are kept.
    fn lista(&mut self, profundidad: u32) -> Result<Valor, ErrorGguf> {
        if profundidad >= MAX_PROFUNDIDAD {
            return Err(ErrorGguf::ListaDemasiadoProfunda);
        }
        let tipo = self.u32()?;
        let longitud = self.u64()?;
        limite("elementos de una lista", longitud, MAX_ELEMENTOS_LISTA)?;
        match tipo {
            8 => {
                for _ in 0..longitud {
                    self.saltar_texto()?;
                }
            }
            9 => {
                for _ in 0..longitud {
                    self.lista(profundidad + 1)?;
                }
            }
            t => match tamano_fijo(t) {
                // `longitud` is already limited, so the multiplication cannot overflow.
                Some(tamano) => self.saltar(longitud * tamano)?,
                None => return Err(ErrorGguf::TipoDesconocido(t)),
            },
        }
        Ok(Valor::Lista { tipo, longitud })
    }
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod pruebas {
    use super::*;

    /// Helper to build test GGUF files in memory.
    #[derive(Default)]
    struct Constructor(Vec<u8>);

    impl Constructor {
        fn u32(&mut self, v: u32) -> &mut Self {
            self.0.extend_from_slice(&v.to_le_bytes());
            self
        }
        fn u64(&mut self, v: u64) -> &mut Self {
            self.0.extend_from_slice(&v.to_le_bytes());
            self
        }
        fn texto(&mut self, t: &str) -> &mut Self {
            self.u64(t.len() as u64);
            self.0.extend_from_slice(t.as_bytes());
            self
        }
        fn clave_u32(&mut self, clave: &str, v: u32) -> &mut Self {
            self.texto(clave).u32(4).u32(v)
        }
    }

    /// A minimal but realistic GGUF: the Qwen2.5 1.5B data and one tensor.
    fn gguf_minimo() -> Vec<u8> {
        let mut c = Constructor::default();
        c.0.extend_from_slice(b"GGUF");
        c.u32(3).u64(1).u64(7); // version, 1 tensor, 7 metadata entries
        c.texto("general.architecture").u32(8).texto("qwen2");
        c.clave_u32("qwen2.block_count", 28);
        c.clave_u32("qwen2.context_length", 32_768);
        c.clave_u32("qwen2.attention.head_count", 12);
        c.clave_u32("qwen2.attention.head_count_kv", 2);
        c.clave_u32("qwen2.embedding_length", 1_536);
        // A list of 2 texts (a tiny vocabulary).
        c.texto("tokenizer.ggml.tokens")
            .u32(9)
            .u32(8)
            .u64(2)
            .texto("ho")
            .texto("la");
        // A tensor: name, 1 dimension of 16, type 0, offset 0.
        c.texto("t").u32(1).u64(16).u32(0).u64(0);
        c.0
    }

    #[test]
    fn lee_una_cabecera_minima() {
        let bytes = gguf_minimo();
        let cabecera = leer(&bytes[..]).expect("debería leerse");
        let r = cabecera.resumen();
        assert_eq!(cabecera.version, 3);
        assert_eq!(cabecera.tensores, 1);
        assert_eq!(cabecera.tipos_tensor, BTreeSet::from([0])); // F32
        assert_eq!(cabecera.bytes_cabecera, bytes.len() as u64);
        assert_eq!(r.arquitectura.as_deref(), Some("qwen2"));
        assert_eq!(r.capas, Some(28));
        assert_eq!(r.contexto_max, Some(32_768));
        assert_eq!(r.dim_clave, Some(128)); // 1536 / 12
        assert_eq!(r.vocabulario, Some(2));
        assert!(!r.plantilla_chat);
    }

    #[test]
    fn estima_la_cache_kv() {
        let cabecera = leer(&gguf_minimo()[..]).unwrap();
        // 4096 × 28 layers × 2 KV heads × (128 + 128) × 2 bytes = 112 MiB, same as in Phase 0.
        assert_eq!(cabecera.cache_kv_bytes(4_096, 2), Some(112 * 1024 * 1024));
    }

    #[test]
    fn rechaza_lo_que_no_es_gguf() {
        let mut bytes = gguf_minimo();
        bytes[..4].copy_from_slice(b"GGML");
        assert!(matches!(leer(&bytes[..]), Err(ErrorGguf::NoEsGguf)));
    }

    #[test]
    fn rechaza_otras_versiones() {
        let mut bytes = gguf_minimo();
        bytes[4..8].copy_from_slice(&2u32.to_le_bytes());
        assert!(matches!(
            leer(&bytes[..]),
            Err(ErrorGguf::VersionNoSoportada(2))
        ));
    }

    #[test]
    fn detecta_archivos_cortados_en_cualquier_punto() {
        // We cut the file at EVERY possible position: it must never "blow up", always a clear error.
        let bytes = gguf_minimo();
        for corte in 0..bytes.len() {
            let resultado = leer(&bytes[..corte]);
            assert!(
                matches!(resultado, Err(ErrorGguf::Truncado)),
                "corte en {corte}: {resultado:?}"
            );
        }
    }

    #[test]
    fn no_reserva_memoria_por_un_texto_gigante() {
        // A file that claims to have an 18-quintillion-byte key.
        let mut c = Constructor::default();
        c.0.extend_from_slice(b"GGUF");
        c.u32(3).u64(0).u64(1).u64(u64::MAX);
        assert!(matches!(
            leer(&c.0[..]),
            Err(ErrorGguf::FueraDeLimite {
                que: "longitud de un texto",
                ..
            })
        ));
    }

    #[test]
    fn rechaza_tipos_desconocidos() {
        let mut c = Constructor::default();
        c.0.extend_from_slice(b"GGUF");
        c.u32(3).u64(0).u64(1).texto("clave").u32(99);
        assert!(matches!(
            leer(&c.0[..]),
            Err(ErrorGguf::TipoDesconocido(99))
        ));
    }

    #[test]
    fn limita_las_listas_anidadas() {
        let mut c = Constructor::default();
        c.0.extend_from_slice(b"GGUF");
        c.u32(3).u64(0).u64(1).texto("clave").u32(9);
        for _ in 0..10 {
            c.u32(9).u64(1); // list of lists of lists…
        }
        assert!(matches!(
            leer(&c.0[..]),
            Err(ErrorGguf::ListaDemasiadoProfunda)
        ));
    }
}
