//! Registros de Locall: lo que **hace** el programa, para poder entender un fallo después.
//!
//! - Un archivo por día: `registros\locall-AAAA-MM-DD.log`, con la hora local. Los de más de `registros_dias`
//!   días se borran al iniciar.
//! - Cada crate escribe con las macros del crate `log` (`log::info!`, `log::warn!`…), sin saber adónde van; aquí
//!   está el "escritor" que las guarda.
//! - Solo se guardan los mensajes de **nuestros crates** (`locall…`) de nivel `INFO` o más importante. Los de otros
//!   crates (por ejemplo, `ureq`) no se guardan.
//!
//! **Privacidad (regla 17, ADR 0014):** se anota lo que hace Locall (modelo cargado, descarga terminada, errores,
//! tiempos), **nunca** el contenido de las conversaciones, las preguntas, las respuestas ni las claves. Esto es una
//! norma para quien escribe `log::info!` en Locall, y una prueba con el motor real comprueba que se cumple.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use log::{Level, LevelFilter, Log, Metadata, Record};

/// Prefijo de los crates cuyos mensajes se guardan.
const PREFIJO_PROPIO: &str = "locall";

/// Lo que puede fallar al iniciar los registros.
#[derive(Debug)]
pub enum ErrorRegistros {
    /// Ya se iniciaron en este proceso (solo puede haber un escritor global).
    YaIniciados,
    /// No se pudo crear la carpeta o limpiar los antiguos.
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

/// Inicia los registros en `carpeta`, borrando antes los de más de `dias` días. Devuelve cuántos se borraron.
///
/// Se llama **una vez**, al arrancar Locall.
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

/// Nombre del archivo de un día.
pub fn nombre_del_dia(fecha: &str) -> String {
    format!("locall-{fecha}.log")
}

/// Borra los registros de Locall (`locall-*.log`) modificados hace más de `dias` días. No toca ningún otro archivo
/// (por ejemplo, el registro del motor).
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

/// Da formato a una línea: `2026-11-08 18:42:03.123 INFO  locall_scheduler: modelo cargado`.
pub fn linea(cuando: &str, nivel: Level, origen: &str, mensaje: &str) -> String {
    // Un mensaje de varias líneas se queda en una sola, para que cada línea del archivo sea un suceso.
    let mensaje = mensaje.replace(['\r', '\n'], " ");
    format!("{cuando} {nivel:<5} {origen}: {mensaje}\n")
}

/// El escritor global.
struct Escritor {
    carpeta: PathBuf,
    /// El archivo del día abierto ahora (fecha, archivo). Con candado: escriben varios hilos.
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

        // Si el disco falla, se pierde la línea: un registro nunca debe hacer fallar a Locall.
        let mut abierto = self
            .abierto
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let hay_que_abrir = abierto.as_ref().is_none_or(|(dia, _)| *dia != fecha);
        if hay_que_abrir {
            // Cambio de día (o primera línea): se abre el archivo del nuevo día.
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
            ("locall-2026-10-01.log", hace(30)), // antiguo: se borra
            ("locall-2026-11-05.log", hace(3)),  // reciente: se queda
            ("motor.log", hace(30)),             // no es nuestro: se queda
            ("notas.txt", hace(30)),             // tampoco
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
