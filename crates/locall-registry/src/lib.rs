//! # locall-registry
//!
//! El **registro de modelos instalados**: una base de datos SQLite (un solo archivo, `locall.db`) donde Locall
//! apunta qué modelo es cada archivo del almacén, de dónde vino, su licencia y cuándo se usó.
//!
//! - **Transacciones:** registrar o eliminar un modelo se hace entero o no se hace (si se va la luz a mitad, no queda
//!   nada a medias).
//! - **Migraciones:** la base de datos lleva un número de versión (`PRAGMA user_version`). Al abrirla, Locall aplica
//!   los cambios que falten, en orden, sin perder datos. Si la creó una versión **más nueva** de Locall, se niega a
//!   tocarla en lugar de estropearla.
//! - **Tipos seguros:** huellas, identificadores y nombres usan los tipos validados de `locall-store`; si alguien
//!   manipulara el archivo a mano, los datos inválidos se detectan al leerlos.
//! - **Fechas** como segundos desde el 1 de enero de 1970 (UTC), sin dependencias extra.

use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use locall_store::{Huella, NombreArchivo, Segmento};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

// ─── Esquema y migraciones ────────────────────────────────────────────────────

/// Cada elemento es una migración: la posición 0 lleva la base de datos de la versión 0 a la 1, la 1 de la 1 a
/// la 2, etc. **Nunca se modifica una migración ya publicada**: los cambios van en una migración nueva al final.
const MIGRACIONES: &[&str] = &[
    // Versión 1: modelos y sus archivos.
    "
    CREATE TABLE modelos (
        id            TEXT    PRIMARY KEY,
        nombre        TEXT    NOT NULL,
        origen        TEXT    NOT NULL,
        revision      TEXT    NOT NULL,
        cuantizacion  TEXT,
        licencia      TEXT,
        arquitectura  TEXT,
        contexto_max  INTEGER,
        verificado    INTEGER NOT NULL CHECK (verificado IN (0, 1)),
        instalado     INTEGER NOT NULL,
        ultimo_uso    INTEGER
    ) STRICT;

    CREATE TABLE archivos (
        modelo_id  TEXT    NOT NULL REFERENCES modelos(id) ON DELETE CASCADE,
        nombre     TEXT    NOT NULL,
        huella     TEXT    NOT NULL CHECK (length(huella) = 64),
        bytes      INTEGER NOT NULL CHECK (bytes >= 0),
        orden      INTEGER NOT NULL,
        PRIMARY KEY (modelo_id, nombre)
    ) STRICT;

    CREATE INDEX archivos_por_huella ON archivos(huella);
    ",
];

/// Versión del esquema que entiende esta versión de Locall.
pub const VERSION_ESQUEMA: u32 = MIGRACIONES.len() as u32;

// ─── Tipos públicos ───────────────────────────────────────────────────────────

/// Un archivo de un modelo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivoModelo {
    pub nombre: NombreArchivo,
    pub huella: Huella,
    pub bytes: u64,
}

/// Los datos para registrar un modelo recién instalado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeloNuevo {
    /// Identificador corto y seguro, por ejemplo `qwen3-4b`.
    pub id: Segmento,
    /// Nombre para mostrar, por ejemplo `Qwen3 4B`.
    pub nombre: String,
    /// De dónde viene, por ejemplo `hf:Qwen/Qwen3-4B-GGUF` o `local`.
    pub origen: String,
    /// Revisión (commit) exacta del origen.
    pub revision: String,
    pub cuantizacion: Option<String>,
    pub licencia: Option<String>,
    pub arquitectura: Option<String>,
    pub contexto_max: Option<u64>,
    /// `true` si viene del catálogo verificado; `false` si se importó del disco.
    pub verificado: bool,
    pub archivos: Vec<ArchivoModelo>,
}

/// Un modelo registrado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Modelo {
    pub id: Segmento,
    pub nombre: String,
    pub origen: String,
    pub revision: String,
    pub cuantizacion: Option<String>,
    pub licencia: Option<String>,
    pub arquitectura: Option<String>,
    pub contexto_max: Option<u64>,
    pub verificado: bool,
    /// Segundos desde 1970 (UTC).
    pub instalado: u64,
    pub ultimo_uso: Option<u64>,
    /// En orden (en un modelo dividido, la parte 1 primero).
    pub archivos: Vec<ArchivoModelo>,
}

impl Modelo {
    /// Lo que ocupan todos sus archivos.
    pub fn bytes_totales(&self) -> u64 {
        self.archivos.iter().map(|a| a.bytes).sum()
    }
}

/// Segundos desde el 1 de enero de 1970 (UTC), la forma en que el registro guarda las fechas.
pub fn ahora() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

// ─── Errores ──────────────────────────────────────────────────────────────────

/// Lo que puede salir mal en el registro.
#[derive(Debug)]
pub enum ErrorRegistro {
    /// Error de la base de datos.
    Sqlite(rusqlite::Error),
    /// Ya hay un modelo con ese identificador.
    YaExiste(String),
    /// No hay ningún modelo con ese identificador.
    NoEncontrado(String),
    /// Un modelo necesita al menos un archivo.
    SinArchivos,
    /// Un dato guardado no es válido (la base de datos se ha modificado a mano o está dañada).
    DatoInvalido {
        campo: &'static str,
        detalle: String,
    },
    /// La base de datos es de una versión más nueva de Locall: no se toca para no estropearla.
    VersionFutura { encontrada: u32, soportada: u32 },
}

impl fmt::Display for ErrorRegistro {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorRegistro::Sqlite(e) => write!(f, "error de la base de datos: {e}"),
            ErrorRegistro::YaExiste(id) => write!(f, "ya hay un modelo llamado «{id}»"),
            ErrorRegistro::NoEncontrado(id) => write!(f, "no hay ningún modelo llamado «{id}»"),
            ErrorRegistro::SinArchivos => write!(f, "un modelo necesita al menos un archivo"),
            ErrorRegistro::DatoInvalido { campo, detalle } => {
                write!(f, "dato no válido en «{campo}»: {detalle}")
            }
            ErrorRegistro::VersionFutura {
                encontrada,
                soportada,
            } => write!(
                f,
                "la base de datos es de una versión más nueva de Locall (esquema {encontrada}, esta versión entiende \
                 hasta el {soportada}); actualiza Locall"
            ),
        }
    }
}

impl std::error::Error for ErrorRegistro {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ErrorRegistro::Sqlite(e) => Some(e),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for ErrorRegistro {
    fn from(e: rusqlite::Error) -> Self {
        ErrorRegistro::Sqlite(e)
    }
}

type Resultado<T> = Result<T, ErrorRegistro>;

// ─── Conversiones entre Rust y SQLite ─────────────────────────────────────────

/// SQLite guarda enteros con signo (i64); nuestros tamaños son u64.
fn a_i64(valor: u64, campo: &'static str) -> Resultado<i64> {
    i64::try_from(valor).map_err(|_| ErrorRegistro::DatoInvalido {
        campo,
        detalle: format!("{valor} es demasiado grande"),
    })
}

fn a_u64(valor: i64, campo: &'static str) -> Resultado<u64> {
    u64::try_from(valor).map_err(|_| ErrorRegistro::DatoInvalido {
        campo,
        detalle: format!("{valor} es negativo"),
    })
}

fn invalido(campo: &'static str, e: impl fmt::Display) -> ErrorRegistro {
    ErrorRegistro::DatoInvalido {
        campo,
        detalle: e.to_string(),
    }
}

// ─── El registro ──────────────────────────────────────────────────────────────

/// El registro de modelos, sobre una base de datos SQLite.
pub struct Registro {
    conexion: Connection,
}

impl Registro {
    /// Abre (o crea) el registro en un archivo y lo actualiza a la versión actual del esquema.
    pub fn abrir(ruta: &Path) -> Resultado<Registro> {
        let conexion = Connection::open(ruta)?;
        // WAL: permite leer mientras otro proceso (la CLI, la app) escribe.
        conexion.pragma_update(None, "journal_mode", "WAL")?;
        Registro::preparar(conexion)
    }

    /// Un registro en memoria (para pruebas): desaparece al cerrarlo.
    pub fn en_memoria() -> Resultado<Registro> {
        Registro::preparar(Connection::open_in_memory()?)
    }

    fn preparar(mut conexion: Connection) -> Resultado<Registro> {
        conexion.pragma_update(None, "foreign_keys", "ON")?; // que ON DELETE CASCADE funcione
        conexion.busy_timeout(Duration::from_secs(5))?; // esperar si otro proceso está escribiendo
        migrar(&mut conexion)?;
        Ok(Registro { conexion })
    }

    /// Versión del esquema de la base de datos abierta.
    pub fn version_esquema(&self) -> Resultado<u32> {
        Ok(self
            .conexion
            .pragma_query_value(None, "user_version", |f| f.get(0))?)
    }

    /// Registra un modelo recién instalado (todo o nada).
    pub fn registrar(&mut self, m: &ModeloNuevo, instalado: u64) -> Resultado<()> {
        if m.archivos.is_empty() {
            return Err(ErrorRegistro::SinArchivos);
        }
        let tx = self.conexion.transaction()?;
        let existe: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM modelos WHERE id = ?1)",
            [m.id.como_str()],
            |f| f.get(0),
        )?;
        if existe {
            return Err(ErrorRegistro::YaExiste(m.id.to_string()));
        }
        tx.execute(
            "INSERT INTO modelos (id, nombre, origen, revision, cuantizacion, licencia, arquitectura,
                                  contexto_max, verificado, instalado, ultimo_uso)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL)",
            params![
                m.id.como_str(),
                m.nombre,
                m.origen,
                m.revision,
                m.cuantizacion,
                m.licencia,
                m.arquitectura,
                m.contexto_max
                    .map(|c| a_i64(c, "contexto_max"))
                    .transpose()?,
                m.verificado,
                a_i64(instalado, "instalado")?,
            ],
        )?;
        // Ordenados por nombre: en un modelo dividido, la parte -00001-of- queda la primera.
        let mut archivos: Vec<&ArchivoModelo> = m.archivos.iter().collect();
        archivos.sort_by(|a, b| a.nombre.cmp(&b.nombre));
        for (orden, a) in archivos.iter().enumerate() {
            tx.execute(
                "INSERT INTO archivos (modelo_id, nombre, huella, bytes, orden) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    m.id.como_str(),
                    a.nombre.como_str(),
                    a.huella.hex(),
                    a_i64(a.bytes, "bytes")?,
                    orden as i64,
                ],
            )?;
        }
        tx.commit()?; // si algo falló antes, `tx` se descarta y no queda nada
        Ok(())
    }

    /// Todos los modelos, ordenados por nombre.
    pub fn listar(&self) -> Resultado<Vec<Modelo>> {
        let ids: Vec<String> = {
            let mut consulta = self
                .conexion
                .prepare("SELECT id FROM modelos ORDER BY nombre, id")?;
            consulta
                .query_map([], |f| f.get(0))?
                .collect::<Result<_, _>>()?
        };
        ids.iter()
            .map(|id| {
                self.leer(id)?
                    .ok_or_else(|| ErrorRegistro::NoEncontrado(id.clone()))
            })
            .collect()
    }

    /// Un modelo por su identificador.
    pub fn obtener(&self, id: &Segmento) -> Resultado<Option<Modelo>> {
        self.leer(id.como_str())
    }

    /// Apunta que el modelo se acaba de usar.
    pub fn marcar_uso(&self, id: &Segmento, momento: u64) -> Resultado<()> {
        let cambiadas = self.conexion.execute(
            "UPDATE modelos SET ultimo_uso = ?1 WHERE id = ?2",
            params![a_i64(momento, "ultimo_uso")?, id.como_str()],
        )?;
        if cambiadas == 0 {
            return Err(ErrorRegistro::NoEncontrado(id.to_string()));
        }
        Ok(())
    }

    /// Elimina un modelo del registro y devuelve las huellas de sus archivos, para que el almacén pueda ofrecer
    /// limpiarlas si ya nadie las usa. (Los archivos del almacén no se tocan aquí.)
    pub fn eliminar(&mut self, id: &Segmento) -> Resultado<Vec<Huella>> {
        let tx = self.conexion.transaction()?;
        let huellas = huellas_de(&tx, id.como_str())?;
        let borradas = tx.execute("DELETE FROM modelos WHERE id = ?1", [id.como_str()])?;
        if borradas == 0 {
            return Err(ErrorRegistro::NoEncontrado(id.to_string()));
        }
        tx.commit()?;
        Ok(huellas)
    }

    /// Todas las huellas que usa algún modelo registrado. Es la segunda comprobación (además del número de
    /// enlaces duros) antes de ofrecer borrar un archivo del almacén.
    pub fn huellas_en_uso(&self) -> Resultado<BTreeSet<Huella>> {
        let mut consulta = self
            .conexion
            .prepare("SELECT DISTINCT huella FROM archivos")?;
        let textos: Vec<String> = consulta
            .query_map([], |f| f.get(0))?
            .collect::<Result<_, _>>()?;
        textos
            .iter()
            .map(|t| Huella::desde_hex(t).map_err(|e| invalido("huella", e)))
            .collect()
    }

    fn leer(&self, id: &str) -> Resultado<Option<Modelo>> {
        let fila = self
            .conexion
            .query_row(
                "SELECT id, nombre, origen, revision, cuantizacion, licencia, arquitectura, contexto_max,
                        verificado, instalado, ultimo_uso
                 FROM modelos WHERE id = ?1",
                [id],
                FilaModelo::desde,
            )
            .optional()?;
        let Some(f) = fila else { return Ok(None) };

        let mut consulta = self.conexion.prepare(
            "SELECT nombre, huella, bytes FROM archivos WHERE modelo_id = ?1 ORDER BY orden",
        )?;
        let crudos: Vec<(String, String, i64)> = consulta
            .query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<_, _>>()?;
        let archivos = crudos
            .into_iter()
            .map(|(nombre, huella, bytes)| {
                Ok(ArchivoModelo {
                    nombre: NombreArchivo::nuevo(&nombre)
                        .map_err(|e| invalido("archivos.nombre", e))?,
                    huella: Huella::desde_hex(&huella)
                        .map_err(|e| invalido("archivos.huella", e))?,
                    bytes: a_u64(bytes, "archivos.bytes")?,
                })
            })
            .collect::<Resultado<Vec<_>>>()?;

        Ok(Some(Modelo {
            id: Segmento::nuevo(&f.id).map_err(|e| invalido("id", e))?,
            nombre: f.nombre,
            origen: f.origen,
            revision: f.revision,
            cuantizacion: f.cuantizacion,
            licencia: f.licencia,
            arquitectura: f.arquitectura,
            contexto_max: f
                .contexto_max
                .map(|c| a_u64(c, "contexto_max"))
                .transpose()?,
            verificado: f.verificado,
            instalado: a_u64(f.instalado, "instalado")?,
            ultimo_uso: f.ultimo_uso.map(|u| a_u64(u, "ultimo_uso")).transpose()?,
            archivos,
        }))
    }
}

/// Una fila de `modelos` tal como sale de SQLite, antes de validarla.
struct FilaModelo {
    id: String,
    nombre: String,
    origen: String,
    revision: String,
    cuantizacion: Option<String>,
    licencia: Option<String>,
    arquitectura: Option<String>,
    contexto_max: Option<i64>,
    verificado: bool,
    instalado: i64,
    ultimo_uso: Option<i64>,
}

impl FilaModelo {
    fn desde(r: &Row<'_>) -> rusqlite::Result<FilaModelo> {
        Ok(FilaModelo {
            id: r.get(0)?,
            nombre: r.get(1)?,
            origen: r.get(2)?,
            revision: r.get(3)?,
            cuantizacion: r.get(4)?,
            licencia: r.get(5)?,
            arquitectura: r.get(6)?,
            contexto_max: r.get(7)?,
            verificado: r.get(8)?,
            instalado: r.get(9)?,
            ultimo_uso: r.get(10)?,
        })
    }
}

fn huellas_de(tx: &Transaction<'_>, id: &str) -> Resultado<Vec<Huella>> {
    let mut consulta =
        tx.prepare("SELECT huella FROM archivos WHERE modelo_id = ?1 ORDER BY orden")?;
    let textos: Vec<String> = consulta
        .query_map([id], |f| f.get(0))?
        .collect::<Result<_, _>>()?;
    textos
        .iter()
        .map(|t| Huella::desde_hex(t).map_err(|e| invalido("archivos.huella", e)))
        .collect()
}

/// Aplica, en orden y cada una en su transacción, las migraciones que falten.
fn migrar(conexion: &mut Connection) -> Resultado<()> {
    let actual: u32 = conexion.pragma_query_value(None, "user_version", |f| f.get(0))?;
    if actual > VERSION_ESQUEMA {
        return Err(ErrorRegistro::VersionFutura {
            encontrada: actual,
            soportada: VERSION_ESQUEMA,
        });
    }
    for (indice, sql) in MIGRACIONES.iter().enumerate().skip(actual as usize) {
        let tx = conexion.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (indice + 1) as u32)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod pruebas;
