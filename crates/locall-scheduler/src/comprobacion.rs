//! Check **before loading** (part 9c): can the engine run this model and does it fit in memory?
//!
//! This way a clear message is shown instantly, instead of waiting seconds for the engine to fail (or, worse, for
//! Windows to start using the disk as memory and the whole computer to stutter).
//!
//! Only the model header is read (a few MB, even if the model takes several GB) with `locall-gguf`, and it is
//! evaluated with `locall-compat` against the available RAM measured by `locall-system`.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use locall_compat::{Encaje, Evaluacion, Problema};
use locall_engine::ErrorMotor;
use locall_gguf::ErrorGguf;

use crate::Carga;

/// What can prevent loading a model with the real starter.
#[derive(Debug)]
pub enum ErrorCarga {
    /// The model header could not be read (damaged, not GGUF…).
    Cabecera(ErrorGguf),
    /// One of the parts of a split model is missing.
    FaltaParte(PathBuf),
    /// The engine cannot run this model.
    Incompatible(Vec<Problema>),
    /// It does not fit in the available memory.
    NoCabe { necesarios: u64, disponibles: u64 },
    /// The system could not be queried (memory, file sizes).
    Sistema(io::Error),
    /// The engine could not start.
    Motor(ErrorMotor),
}

impl fmt::Display for ErrorCarga {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // The technical detail (`ErrorGguf`) stays in `source()`, for the logs.
            ErrorCarga::Cabecera(_) => write!(
                f,
                "el archivo del modelo no es válido o está incompleto; vuelve a descargarlo"
            ),
            ErrorCarga::FaltaParte(ruta) => {
                let nombre = ruta.file_name().unwrap_or_default().to_string_lossy();
                write!(
                    f,
                    "falta una parte del modelo ({nombre}); vuelve a descargarlo"
                )
            }
            ErrorCarga::Incompatible(problemas) => {
                write!(f, "este modelo no se puede usar: ")?;
                for (i, p) in problemas.iter().enumerate() {
                    if i > 0 {
                        write!(f, "; ")?;
                    }
                    write!(f, "{p}")?;
                }
                Ok(())
            }
            ErrorCarga::NoCabe {
                necesarios,
                disponibles,
            } => write!(
                f,
                "este modelo necesita unos {} y ahora hay {} libres; cierra otros programas, reduce el contexto o \
                 elige un modelo más pequeño",
                gb(*necesarios),
                gb(*disponibles)
            ),
            ErrorCarga::Sistema(e) => write!(f, "no se pudo consultar el equipo: {e}"),
            ErrorCarga::Motor(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ErrorCarga {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ErrorCarga::Cabecera(e) => Some(e),
            ErrorCarga::Sistema(e) => Some(e),
            ErrorCarga::Motor(e) => Some(e),
            _ => None,
        }
    }
}

impl From<ErrorMotor> for ErrorCarga {
    fn from(e: ErrorMotor) -> Self {
        ErrorCarga::Motor(e)
    }
}

/// "4.9 GB" (in decimal GB, as Windows shows them in most places).
fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

/// Evaluates the load with the RAM available **now**. Also lets the app warn before requesting it (for example,
/// showing "it will be tight" with [`Encaje::Justo`]).
pub fn evaluar_carga(carga: &Carga) -> Result<Evaluacion, ErrorCarga> {
    let disponibles = locall_system::memoria()
        .map_err(ErrorCarga::Sistema)?
        .disponible;
    evaluar_con_memoria(carga, disponibles)
}

/// Like [`evaluar_carga`], with the given available memory (so the "does not fit" case can be tested).
pub(crate) fn evaluar_con_memoria(
    carga: &Carga,
    disponibles: u64,
) -> Result<Evaluacion, ErrorCarga> {
    let principal = &carga.config.modelo;
    let cabecera = locall_gguf::leer_archivo(principal).map_err(ErrorCarga::Cabecera)?;
    let mut bytes = 0u64;
    for parte in archivos_del_modelo(principal, cabecera.resumen().partes) {
        let tamano = match std::fs::metadata(&parte) {
            Ok(m) => m.len(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(ErrorCarga::FaltaParte(parte));
            }
            Err(e) => return Err(ErrorCarga::Sistema(e)),
        };
        bytes = bytes.saturating_add(tamano);
    }
    Ok(locall_compat::evaluar(
        &cabecera,
        bytes,
        u64::from(carga.config.contexto),
        disponibles,
    ))
}

/// Decides whether it can be loaded according to the evaluation: compatibility problems and [`Encaje::NoCabe`]
/// prevent it. [`Encaje::Justo`] is allowed (D7: a model that only just fits is a valid option the user chooses).
pub(crate) fn decidir(evaluacion: &Evaluacion, disponibles: u64) -> Result<(), ErrorCarga> {
    if !evaluacion.comprobacion.es_compatible() {
        return Err(ErrorCarga::Incompatible(
            evaluacion.comprobacion.problemas.clone(),
        ));
    }
    if evaluacion.encaje == Encaje::NoCabe {
        return Err(ErrorCarga::NoCabe {
            necesarios: evaluacion.estimacion.total,
            disponibles,
        });
    }
    Ok(())
}

/// All the files of a model. If it is split (`partes` > 1) and the name follows llama.cpp's official format
/// (`name-00001-of-00003.gguf`), the other parts are deduced; otherwise, only the main one.
pub(crate) fn archivos_del_modelo(principal: &Path, partes: Option<u64>) -> Vec<PathBuf> {
    let unico = vec![principal.to_path_buf()];
    let Some(n) = partes.filter(|&n| n > 1) else {
        return unico;
    };
    let Some(nombre) = principal.file_name().and_then(|n| n.to_str()) else {
        return unico;
    };
    let sufijo = format!("-00001-of-{n:05}.gguf");
    let Some(base) = nombre.strip_suffix(&sufijo) else {
        return unico;
    };
    (1..=n)
        .map(|i| principal.with_file_name(format!("{base}-{i:05}-of-{n:05}.gguf")))
        .collect()
}
