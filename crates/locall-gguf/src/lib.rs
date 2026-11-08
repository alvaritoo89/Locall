//! # locall-gguf
//!
//! Lector de **cabeceras GGUF** (versión 3), el formato de los modelos de llama.cpp.
//!
//! Lee la "etiqueta" del modelo (metadatos y descripción de los tensores) **sin cargar los pesos**, para:
//! - conocer la arquitectura, el número de capas, el contexto máximo, etc.;
//! - comprobar la compatibilidad antes de cargar un modelo;
//! - estimar la memoria de la caché KV (ver ADR 0005).
//!
//! Formato (según `ggml/include/gguf.h` de llama.cpp), todo en *little-endian*:
//! 1. Firma `GGUF` (4 bytes) y versión (`u32`).
//! 2. Número de tensores y de pares clave-valor (`i64`).
//! 3. Los pares clave-valor (metadatos).
//! 4. La descripción de cada tensor (nombre, dimensiones, tipo, posición).
//!
//! ## Seguridad
//!
//! Un archivo GGUF descargado es **entrada no confiable**: podría estar dañado o manipulado. Por eso cada longitud
//! leída del archivo se comprueba contra un **límite** antes de reservar memoria, y los errores se devuelven como
//! [`ErrorGguf`] en lugar de provocar un cierre del programa.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;

/// Firma con la que empieza todo archivo GGUF.
pub const MAGIA: [u8; 4] = *b"GGUF";
/// Única versión del formato que admitimos (ADR 0004).
pub const VERSION_SOPORTADA: u32 = 3;

// ─── Límites de seguridad ─────────────────────────────────────────────────────
// Holgados para modelos reales (la cabecera de un Qwen ocupa ~6 MB), pero impiden que un archivo malicioso nos
// haga reservar gigas de memoria o recorrer listas infinitas.

/// Longitud máxima de un texto (claves, plantillas de chat…).
const MAX_TEXTO: u64 = 64 * 1024 * 1024;
/// Número máximo de pares clave-valor.
const MAX_METADATOS: u64 = 100_000;
/// Número máximo de tensores.
const MAX_TENSORES: u64 = 1_000_000;
/// Número máximo de dimensiones de un tensor (llama.cpp usa 4).
const MAX_DIMENSIONES: u64 = 8;
/// Número máximo de elementos de una lista (el vocabulario ronda los 150.000).
const MAX_ELEMENTOS_LISTA: u64 = 100_000_000;
/// Profundidad máxima de listas dentro de listas.
const MAX_PROFUNDIDAD: u32 = 4;

// ─── Tipos públicos ───────────────────────────────────────────────────────────

/// Un valor de los metadatos.
///
/// Las listas no guardan su contenido (el vocabulario ocupa varios MB): solo su tipo y su longitud.
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
    /// Devuelve el valor como número entero sin signo, si lo es (y no es negativo).
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

    /// Devuelve el valor como texto, si lo es.
    pub fn como_texto(&self) -> Option<&str> {
        match self {
            Valor::Texto(t) => Some(t),
            _ => None,
        }
    }
}

/// Lo que puede salir mal al leer una cabecera.
#[derive(Debug)]
pub enum ErrorGguf {
    /// Error de lectura del sistema (archivo inexistente, sin permisos…).
    Io(io::Error),
    /// El archivo no empieza por `GGUF`.
    NoEsGguf,
    /// Versión del formato distinta de la 3.
    VersionNoSoportada(u32),
    /// El archivo se acaba antes de lo que dice su propia cabecera.
    Truncado,
    /// Un tipo de valor que no existe en el formato.
    TipoDesconocido(u32),
    /// Un texto que no es UTF-8 válido.
    TextoNoUtf8,
    /// Una cantidad leída del archivo supera nuestros límites de seguridad.
    FueraDeLimite {
        que: &'static str,
        valor: u64,
        maximo: u64,
    },
    /// Listas anidadas más allá de lo razonable.
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
        // "Fin de archivo inesperado" significa, para nosotros, un archivo truncado.
        if e.kind() == io::ErrorKind::UnexpectedEof {
            ErrorGguf::Truncado
        } else {
            ErrorGguf::Io(e)
        }
    }
}

/// La cabecera de un modelo GGUF.
#[derive(Debug, Clone)]
pub struct Cabecera {
    pub version: u32,
    pub tensores: u64,
    pub metadatos: BTreeMap<String, Valor>,
    /// Bytes que ocupa la cabecera completa (metadatos + descripción de tensores).
    pub bytes_cabecera: u64,
    /// Tipos de datos (cuantización) que usan los tensores, por su número en ggml (`ggml_type`).
    /// Sirve para comprobar que el motor sabe leerlos.
    pub tipos_tensor: BTreeSet<u32>,
}

/// Los datos de la cabecera que más interesan a Locall.
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
    /// Número de archivos si el modelo está dividido (`split.count`).
    pub partes: Option<u64>,
}

impl Cabecera {
    /// La arquitectura del modelo (`general.architecture`), por ejemplo `"qwen2"`.
    pub fn arquitectura(&self) -> Option<&str> {
        self.metadatos.get("general.architecture")?.como_texto()
    }

    /// Lee un número de las claves propias de la arquitectura: `"<arquitectura>.<sufijo>"`.
    fn de_arquitectura(&self, sufijo: &str) -> Option<u64> {
        let arquitectura = self.arquitectura()?;
        self.metadatos
            .get(&format!("{arquitectura}.{sufijo}"))?
            .como_u64()
    }

    /// Reúne los datos principales en un [`Resumen`].
    pub fn resumen(&self) -> Resumen {
        let cabezas = self.de_arquitectura("attention.head_count");
        let embebido = self.de_arquitectura("embedding_length");
        // Si el modelo no indica la dimensión de cada cabeza, se calcula: embebido / cabezas.
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

    /// Estima los bytes de la caché KV para `contexto` tokens.
    ///
    /// Fórmula: contexto × capas × cabezas_kv × (dim_clave + dim_valor) × bytes_por_elemento.
    /// Con la caché en `f16`, `bytes_por_elemento` es 2. Devuelve `None` si faltan datos o hay desbordamiento.
    /// Es una aproximación: no contempla atención por ventana (SWA) ni modelos recurrentes.
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

// ─── Lectura ──────────────────────────────────────────────────────────────────

/// Lee una cabecera GGUF desde cualquier fuente de bytes (archivo, memoria, red…).
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

    // De la descripción de los tensores solo guardamos su tipo; hay que recorrerla entera para saber dónde acaba
    // la cabecera y comprobar que el archivo está completo.
    let mut tipos_tensor = BTreeSet::new();
    for _ in 0..tensores {
        l.saltar_texto()?; // nombre
        let dimensiones = u64::from(l.u32()?);
        limite("dimensiones de un tensor", dimensiones, MAX_DIMENSIONES)?;
        l.saltar(8 * dimensiones)?; // tamaño de cada dimensión (i64)
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

/// Abre un archivo y lee su cabecera.
pub fn leer_archivo(ruta: impl AsRef<Path>) -> Result<Cabecera, ErrorGguf> {
    let archivo = File::open(ruta)?;
    leer(BufReader::new(archivo))
}

/// Devuelve un error si `valor` supera `maximo`.
fn limite(que: &'static str, valor: u64, maximo: u64) -> Result<(), ErrorGguf> {
    if valor > maximo {
        Err(ErrorGguf::FueraDeLimite { que, valor, maximo })
    } else {
        Ok(())
    }
}

/// Bytes que ocupa un elemento de tipo fijo (`None` para textos y listas, que tienen longitud variable).
fn tamano_fijo(tipo: u32) -> Option<u64> {
    match tipo {
        0 | 1 | 7 => Some(1), // u8, i8, bool
        2 | 3 => Some(2),     // u16, i16
        4..=6 => Some(4),     // u32, i32, f32
        10..=12 => Some(8),   // u64, i64, f64
        _ => None,
    }
}

/// Envoltorio que lee de la fuente y cuenta cuántos bytes lleva leídos.
struct Lector<R> {
    interno: R,
    leidos: u64,
}

impl<R: Read> Lector<R> {
    /// Lee exactamente `N` bytes. `const N: usize` es un parámetro que se fija al compilar.
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

    /// Avanza `n` bytes sin guardarlos (sin reservar memoria).
    fn saltar(&mut self, n: u64) -> Result<(), ErrorGguf> {
        let copiados = io::copy(&mut (&mut self.interno).take(n), &mut io::sink())?;
        if copiados < n {
            return Err(ErrorGguf::Truncado);
        }
        self.leidos += n;
        Ok(())
    }

    /// Lee un texto: longitud (`u64`) + bytes UTF-8. La longitud se limita ANTES de reservar memoria.
    fn texto(&mut self) -> Result<String, ErrorGguf> {
        let longitud = self.u64()?;
        limite("longitud de un texto", longitud, MAX_TEXTO)?;
        let mut bytes = vec![0u8; longitud as usize];
        self.interno.read_exact(&mut bytes)?;
        self.leidos += longitud;
        String::from_utf8(bytes).map_err(|_| ErrorGguf::TextoNoUtf8)
    }

    /// Salta un texto sin guardarlo (para los nombres de tensores y los textos de las listas).
    fn saltar_texto(&mut self) -> Result<(), ErrorGguf> {
        let longitud = self.u64()?;
        limite("longitud de un texto", longitud, MAX_TEXTO)?;
        self.saltar(longitud)
    }

    /// Lee un valor del tipo indicado.
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

    /// Lee una lista: tipo de sus elementos (`u32`) + número de elementos (`u64`) + elementos.
    /// Los elementos se saltan; solo se guarda el tipo y la longitud.
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
                // `longitud` ya está limitada, así que la multiplicación no puede desbordar.
                Some(tamano) => self.saltar(longitud * tamano)?,
                None => return Err(ErrorGguf::TipoDesconocido(t)),
            },
        }
        Ok(Valor::Lista { tipo, longitud })
    }
}

// ─── Pruebas unitarias ────────────────────────────────────────────────────────

#[cfg(test)]
mod pruebas {
    use super::*;

    /// Ayudante para construir archivos GGUF de prueba en memoria.
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

    /// Un GGUF mínimo pero realista: los datos de Qwen2.5 1.5B y un tensor.
    fn gguf_minimo() -> Vec<u8> {
        let mut c = Constructor::default();
        c.0.extend_from_slice(b"GGUF");
        c.u32(3).u64(1).u64(7); // versión, 1 tensor, 7 metadatos
        c.texto("general.architecture").u32(8).texto("qwen2");
        c.clave_u32("qwen2.block_count", 28);
        c.clave_u32("qwen2.context_length", 32_768);
        c.clave_u32("qwen2.attention.head_count", 12);
        c.clave_u32("qwen2.attention.head_count_kv", 2);
        c.clave_u32("qwen2.embedding_length", 1_536);
        // Lista de 2 textos (un vocabulario en miniatura).
        c.texto("tokenizer.ggml.tokens")
            .u32(9)
            .u32(8)
            .u64(2)
            .texto("ho")
            .texto("la");
        // Un tensor: nombre, 1 dimensión de 16, tipo 0, posición 0.
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
        // 4096 × 28 capas × 2 cabezas KV × (128 + 128) × 2 bytes = 112 MiB, igual que en la Fase 0.
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
        // Cortamos el archivo en TODAS las posiciones posibles: nunca debe "explotar", siempre un error claro.
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
        // Un archivo que dice tener una clave de 18 trillones de bytes.
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
            c.u32(9).u64(1); // lista de listas de listas…
        }
        assert!(matches!(
            leer(&c.0[..]),
            Err(ErrorGguf::ListaDemasiadoProfunda)
        ));
    }
}
