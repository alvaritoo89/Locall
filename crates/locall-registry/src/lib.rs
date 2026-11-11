//! # locall-registry
//!
//! The **registry of installed models**: a SQLite database (a single file, `locall.db`) where Locall records which
//! model each store file belongs to, where it came from, its license and when it was used.
//!
//! - **Transactions:** registering or removing a model happens entirely or not at all (if the power goes out
//!   halfway, nothing is left half-done).
//! - **Migrations:** the database carries a version number (`PRAGMA user_version`). When opening it, Locall applies
//!   the missing changes, in order, without losing data. If a **newer** version of Locall created it, it refuses to
//!   touch it instead of breaking it.
//! - **Safe types:** hashes, identifiers and names use the validated types from `locall-store`; if someone tampered
//!   with the file by hand, invalid data is detected when it is read.
//! - **Dates** as seconds since January 1, 1970 (UTC), with no extra dependencies.

use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use locall_store::{Huella, NombreArchivo, Segmento};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

// ─── Schema and migrations ────────────────────────────────────────────────────

/// Each element is a migration: position 0 takes the database from version 0 to 1, position 1 from 1 to 2, etc.
/// **A published migration is never modified**: changes go into a new migration at the end.
const MIGRACIONES: &[&str] = &[
    // Version 1: models and their files.
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
    // Version 2: saved conversations and their messages (Phase 5, part 5d).
    "
    CREATE TABLE conversaciones (
        id           INTEGER PRIMARY KEY,
        titulo       TEXT    NOT NULL,
        modelo       TEXT,
        sistema      TEXT    NOT NULL DEFAULT '',
        temperatura  REAL,
        creada       INTEGER NOT NULL,
        actualizada  INTEGER NOT NULL
    ) STRICT;

    CREATE TABLE mensajes (
        conversacion_id  INTEGER NOT NULL REFERENCES conversaciones(id) ON DELETE CASCADE,
        orden            INTEGER NOT NULL,
        rol              TEXT    NOT NULL CHECK (rol IN ('user', 'assistant')),
        contenido        TEXT    NOT NULL,
        razonamiento     TEXT    NOT NULL DEFAULT '',
        modelo           TEXT,
        tokens           INTEGER,
        velocidad        REAL,
        cortada          INTEGER NOT NULL DEFAULT 0 CHECK (cortada IN (0, 1)),
        creado           INTEGER NOT NULL,
        PRIMARY KEY (conversacion_id, orden)
    ) STRICT;

    CREATE INDEX conversaciones_por_fecha ON conversaciones(actualizada);
    ",
];

/// Schema version this version of Locall understands.
pub const VERSION_ESQUEMA: u32 = MIGRACIONES.len() as u32;

// ─── Public types ─────────────────────────────────────────────────────────────

/// A file of a model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivoModelo {
    pub nombre: NombreArchivo,
    pub huella: Huella,
    pub bytes: u64,
}

/// The data to register a freshly installed model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeloNuevo {
    /// Short, safe identifier, for example `qwen3-4b`.
    pub id: Segmento,
    /// Display name, for example `Qwen3 4B`.
    pub nombre: String,
    /// Where it comes from, for example `hf:Qwen/Qwen3-4B-GGUF` or `local`.
    pub origen: String,
    /// Exact revision (commit) of the source.
    pub revision: String,
    pub cuantizacion: Option<String>,
    pub licencia: Option<String>,
    pub arquitectura: Option<String>,
    pub contexto_max: Option<u64>,
    /// `true` if it comes from the verified catalog; `false` if it was imported from disk.
    pub verificado: bool,
    pub archivos: Vec<ArchivoModelo>,
}

/// A registered model.
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
    /// Seconds since 1970 (UTC).
    pub instalado: u64,
    pub ultimo_uso: Option<u64>,
    /// In order (in a split model, part 1 first).
    pub archivos: Vec<ArchivoModelo>,
}

impl Modelo {
    /// The space all its files take.
    pub fn bytes_totales(&self) -> u64 {
        self.archivos.iter().map(|a| a.bytes).sum()
    }
}

/// Seconds since January 1, 1970 (UTC), the way the registry stores dates.
pub fn ahora() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

// ─── Errors ───────────────────────────────────────────────────────────────────

/// What can go wrong in the registry.
#[derive(Debug)]
pub enum ErrorRegistro {
    /// Database error.
    Sqlite(rusqlite::Error),
    /// There is already a model with that identifier.
    YaExiste(String),
    /// There is no model with that identifier.
    NoEncontrado(String),
    /// A model needs at least one file.
    SinArchivos,
    /// A stored value is not valid (the database was modified by hand or is damaged).
    DatoInvalido {
        campo: &'static str,
        detalle: String,
    },
    /// The database belongs to a newer version of Locall: it is not touched so as not to break it.
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

// ─── Conversions between Rust and SQLite ──────────────────────────────────────

/// SQLite stores signed integers (i64); our sizes are u64.
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

// ─── The registry ─────────────────────────────────────────────────────────────

/// The model registry, on top of a SQLite database.
pub struct Registro {
    conexion: Connection,
}

impl Registro {
    /// Opens (or creates) the registry in a file and upgrades it to the current schema version.
    pub fn abrir(ruta: &Path) -> Resultado<Registro> {
        let conexion = Connection::open(ruta)?;
        // WAL: allows reading while another process (the CLI, the app) writes.
        conexion.pragma_update(None, "journal_mode", "WAL")?;
        Registro::preparar(conexion)
    }

    /// An in-memory registry (for tests): it disappears when closed.
    pub fn en_memoria() -> Resultado<Registro> {
        Registro::preparar(Connection::open_in_memory()?)
    }

    fn preparar(mut conexion: Connection) -> Resultado<Registro> {
        conexion.pragma_update(None, "foreign_keys", "ON")?; // so that ON DELETE CASCADE works
        // Deleted data is filled with zeros: a deleted conversation must not be recoverable from the file.
        conexion.pragma_update(None, "secure_delete", "ON")?;
        conexion.busy_timeout(Duration::from_secs(5))?; // wait if another process is writing
        migrar(&mut conexion)?;
        Ok(Registro { conexion })
    }

    /// Schema version of the open database.
    pub fn version_esquema(&self) -> Resultado<u32> {
        Ok(self
            .conexion
            .pragma_query_value(None, "user_version", |f| f.get(0))?)
    }

    /// Registers a freshly installed model (all or nothing).
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
        // Sorted by name: in a split model, the -00001-of- part comes first.
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
        tx.commit()?; // if something failed earlier, `tx` is dropped and nothing remains
        Ok(())
    }

    /// All the models, sorted by name.
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

    /// A model by its identifier.
    pub fn obtener(&self, id: &Segmento) -> Resultado<Option<Modelo>> {
        self.leer(id.como_str())
    }

    /// Records that the model was just used.
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

    /// Removes a model from the registry and returns the hashes of its files, so the store can offer to clean them up
    /// if nobody uses them anymore. (The store files are not touched here.)
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

    /// All the hashes used by some registered model. It is the second check (besides the hard link count) before
    /// offering to delete a file from the store.
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

/// A `modelos` row as it comes out of SQLite, before validating it.
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

/// Applies the missing migrations, in order and each in its own transaction.
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

mod conversaciones;
#[cfg(test)]
mod pruebas;

pub use conversaciones::{
    Autor, Conversacion, MensajeGuardado, MensajeNuevo, TITULO_MAX, titulo_valido,
};
