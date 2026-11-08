//! Comprobación **antes de cargar** (parte 9c): ¿el motor sabe ejecutar este modelo y cabe en la memoria?
//!
//! Así se avisa con un mensaje claro en un instante, en vez de esperar segundos a que el motor falle (o, peor, a que
//! Windows empiece a usar el disco como memoria y todo el equipo vaya a trompicones).
//!
//! Solo se lee la cabecera del modelo (unos MB, aunque el modelo ocupe varios GB) con `locall-gguf`, y se evalúa con
//! `locall-compat` frente a la RAM disponible que mide `locall-system`.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use locall_compat::{Encaje, Evaluacion, Problema};
use locall_engine::ErrorMotor;
use locall_gguf::ErrorGguf;

use crate::Carga;

/// Lo que puede impedir cargar un modelo con el arrancador real.
#[derive(Debug)]
pub enum ErrorCarga {
    /// No se pudo leer la cabecera del modelo (dañado, no es GGUF…).
    Cabecera(ErrorGguf),
    /// Falta una de las partes de un modelo dividido.
    FaltaParte(PathBuf),
    /// El motor no sabe ejecutar este modelo.
    Incompatible(Vec<Problema>),
    /// No cabe en la memoria disponible.
    NoCabe { necesarios: u64, disponibles: u64 },
    /// No se pudo consultar el sistema (memoria, tamaño de archivos).
    Sistema(io::Error),
    /// El motor no pudo arrancar.
    Motor(ErrorMotor),
}

impl fmt::Display for ErrorCarga {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // El detalle técnico (`ErrorGguf`) queda en `source()`, para los registros.
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

/// "4.9 GB" (en GB decimales, como los muestra Windows en la mayoría de sitios).
fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

/// Evalúa la carga con la RAM disponible **ahora**. Sirve también para que la app avise antes de pedirla
/// (por ejemplo, mostrar "irá justo" con [`Encaje::Justo`]).
pub fn evaluar_carga(carga: &Carga) -> Result<Evaluacion, ErrorCarga> {
    let disponibles = locall_system::memoria()
        .map_err(ErrorCarga::Sistema)?
        .disponible;
    evaluar_con_memoria(carga, disponibles)
}

/// Como [`evaluar_carga`], con la memoria disponible indicada (para poder probar el caso "no cabe").
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

/// Decide si se puede cargar según la evaluación: los problemas de compatibilidad y [`Encaje::NoCabe`] lo impiden.
/// [`Encaje::Justo`] se permite (D7: el 7B en 16 GB "al límite" es una opción válida que el usuario elige).
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

/// Todos los archivos de un modelo. Si está dividido (`partes` > 1) y el nombre sigue el formato oficial de
/// llama.cpp (`nombre-00001-of-00003.gguf`), se deducen las demás partes; si no, solo el principal.
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
