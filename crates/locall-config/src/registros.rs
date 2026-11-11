//! Locall's logs: what the program **does**, so a failure can be understood afterwards.
//!
//! - One file per day: `registros\locall-YYYY-MM-DD.log`, in local time. Those older than `registros_dias` days are
//!   deleted on startup.
//! - Each crate writes with the `log` crate's macros (`log::info!`, `log::warn!`…), without knowing where they go;
//!   the "writer" that saves them is here.
//! - Only messages from **our crates** (`locall…`) at `INFO` level or above are saved. Those from other crates (for
//!   example, `ureq`) are not.
//!
//! **Privacy (rule 17, ADR 0014):** what Locall does is recorded (model loaded, download finished, errors, timings),
//! **never** the content of conversations, questions, answers or keys. This is a rule for whoever writes
//! `log::info!` in Locall, and a test with the real engine checks it is followed.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use log::{Level, LevelFilter, Log, Metadata, Record};

/// Prefix of the crates whose messages are saved.
const PREFIJO_PROPIO: &str = "locall";

/// What can fail when starting the logs.
#[derive(Debug)]
pub enum ErrorRegistros {
    /// They were already started in this process (there can only be one global writer).
    YaIniciados,
    /// The folder could not be created or the old ones could not be cleaned.
    Io(io::Error),
}

impl std::fmt::Display for ErrorRegistros {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErrorRegistros::YaIniciados => write!(f, "los registros ya estaban iniciados"),
            ErrorRegistros::Io(e) => write!(f, "no se pudieron preparar los registros: {e}"),
        }
    }
}

impl std::error::Error for ErrorRegistros {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ErrorRegistros::Io(e) => Some(e),
            ErrorRegistros::YaIniciados => None,
        }
    }
}

/// Starts the logs in `carpeta`, first deleting those older than `dias` days. Returns how many were deleted.
///
/// Called **once**, when Locall starts.
pub fn iniciar(carpeta: &Path, dias: u32) -> Result<usize, ErrorRegistros> {
    fs::create_dir_all(carpeta).map_err(ErrorRegistros::Io)?;
    let borrados = limpiar(carpeta, dias, SystemTime::now()).map_err(ErrorRegistros::Io)?;
    let escritor = Escritor {
        carpeta: carpeta.to_path_buf(),
        abierto: Mutex::new(None),
    };
    log::set_boxed_logger(Box::new(escritor)).map_err(|_| ErrorRegistros::YaIniciados)?;
    log::set_max_level(LevelFilter::Info);
    Ok(borrados)
}

/// File name for a day.
pub fn nombre_del_dia(fecha: &str) -> String {
    format!("locall-{fecha}.log")
}

/// Deletes Locall's logs (`locall-*.log`) modified more than `dias` days ago. It does not touch any other file (for
/// example, the engine's log).
pub fn limpiar(carpeta: &Path, dias: u32, ahora: SystemTime) -> io::Result<usize> {
    let limite = Duration::from_secs(u64::from(dias) * 24 * 60 * 60);
    let mut borrados = 0;
    for entrada in fs::read_dir(carpeta)? {
        let entrada = entrada?;
        let nombre = entrada.file_name();
        let Some(nombre) = nombre.to_str() else {
            continue;
        };
        if !(nombre.starts_with("locall-") && nombre.ends_with(".log")) {
            continue;
        }
        let Ok(modificado) = entrada.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        let antiguo = ahora
            .duration_since(modificado)
            .is_ok_and(|edad| edad > limite);
        if antiguo && fs::remove_file(entrada.path()).is_ok() {
            borrados += 1;
        }
    }
    Ok(borrados)
}

/// Formats a line: `2026-11-08 18:42:03.123 INFO  locall_scheduler: modelo cargado`.
pub fn linea(cuando: &str, nivel: Level, origen: &str, mensaje: &str) -> String {
    // A multi-line message is kept on a single line, so each line of the file is one event.
    let mensaje = mensaje.replace(['\r', '\n'], " ");
    format!("{cuando} {nivel:<5} {origen}: {mensaje}\n")
}

/// The global writer.
struct Escritor {
    carpeta: PathBuf,
    /// The day's file currently open (date, file). Behind a lock: several threads write.
    abierto: Mutex<Option<(String, File)>>,
}

impl Log for Escritor {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Info && metadata.target().starts_with(PREFIJO_PROPIO)
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let (cuando, fecha) = match locall_system::fecha_hora_local() {
            Ok(ahora) => (ahora.to_string(), ahora.fecha()),
            Err(_) => ("sin-hora".to_owned(), "sin-fecha".to_owned()),
        };
        let texto = linea(
            &cuando,
            record.level(),
            record.target(),
            &record.args().to_string(),
        );

        // If the disk fails, the line is lost: a log must never make Locall fail.
        let mut abierto = self
            .abierto
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let hay_que_abrir = abierto.as_ref().is_none_or(|(dia, _)| *dia != fecha);
        if hay_que_abrir {
            // Day change (or first line): the new day's file is opened.
            *abierto = OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.carpeta.join(nombre_del_dia(&fecha)))
                .ok()
                .map(|archivo| (fecha, archivo));
        }
        if let Some((_, archivo)) = abierto.as_mut() {
            let _ = archivo.write_all(texto.as_bytes());
        }
    }

    fn flush(&self) {
        if let Some((_, archivo)) = self
            .abierto
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_mut()
        {
            let _ = archivo.flush();
        }
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn da_formato_a_una_linea() {
        assert_eq!(
            linea(
                "2026-11-08 18:42:03.123",
                Level::Info,
                "locall_scheduler",
                "modelo cargado"
            ),
            "2026-11-08 18:42:03.123 INFO  locall_scheduler: modelo cargado\n"
        );
        assert_eq!(
            linea("t", Level::Warn, "x", "dos\r\nlíneas"),
            "t WARN  x: dos  líneas\n"
        );
    }

    #[test]
    fn limpia_solo_los_registros_antiguos_de_locall() {
        let carpeta =
            std::env::temp_dir().join(format!("locall-reg-limpiar-{}", std::process::id()));
        let _ = fs::remove_dir_all(&carpeta);
        fs::create_dir_all(&carpeta).unwrap();
        let ahora = SystemTime::now();
        let hace = |dias: u64| ahora - Duration::from_secs(dias * 24 * 60 * 60);
        let archivos = [
            ("locall-2026-10-01.log", hace(30)), // old: deleted
            ("locall-2026-11-05.log", hace(3)),  // recent: kept
            ("motor.log", hace(30)),             // not ours: kept
            ("notas.txt", hace(30)),             // neither
        ];
        for (nombre, cuando) in archivos {
            let ruta = carpeta.join(nombre);
            fs::write(&ruta, b"x").unwrap();
            File::options()
                .write(true)
                .open(&ruta)
                .unwrap()
                .set_modified(cuando)
                .unwrap();
        }

        assert_eq!(limpiar(&carpeta, 7, ahora).unwrap(), 1);
        assert!(!carpeta.join("locall-2026-10-01.log").exists());
        assert!(carpeta.join("locall-2026-11-05.log").exists());
        assert!(carpeta.join("motor.log").exists());
        assert!(carpeta.join("notas.txt").exists());
        let _ = fs::remove_dir_all(&carpeta);
    }
}
