//! Helpers shared by the tests with the real engine.
//!
//! If the files are not there, the tests are skipped (with a notice). Default paths, changeable with environment
//! variables:
//! - `LOCALL_MOTOR`: `C:\Locall-poc\motor\cpu\llama-server.exe`
//! - `LOCALL_MODELOS`: `C:\Locall-poc\modelos` (uses `qwen2.5-1.5b-instruct-q4_k_m.gguf`)

#![allow(dead_code)] // each test file uses only some of these helpers

use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use locall_engine::{ConfigMotor, Motor};

pub const MODELO: &str = "qwen2.5-1.5b-instruct-q4_k_m.gguf";
pub const CARGA_MAXIMA: Duration = Duration::from_secs(120);

/// Configuration for the tests, or `None` if the engine or the model is missing.
pub fn config(nombre: &str) -> Option<ConfigMotor> {
    let motor = std::env::var_os("LOCALL_MOTOR").map_or_else(
        || PathBuf::from(r"C:\Locall-poc\motor\cpu\llama-server.exe"),
        PathBuf::from,
    );
    let modelos = std::env::var_os("LOCALL_MODELOS")
        .map_or_else(|| PathBuf::from(r"C:\Locall-poc\modelos"), PathBuf::from);
    let modelo = modelos.join(MODELO);
    if !motor.is_file() || !modelo.is_file() {
        eprintln!(
            "(prueba saltada: faltan {} o {})",
            motor.display(),
            modelo.display()
        );
        return None;
    }
    Some(ConfigMotor {
        ejecutable: motor,
        modelo,
        contexto: 2048,
        hilos: locall_system::hilos_para_motor(),
        capas_gpu: 0,
        paralelo: 1,
        registro: std::env::temp_dir()
            .join(format!("locall-motor-{nombre}-{}.log", std::process::id())),
    })
}

/// Is anything listening on that port?
pub fn escucha(puerto: u16) -> bool {
    TcpStream::connect_timeout(&([127, 0, 0, 1], puerto).into(), Duration::from_millis(500)).is_ok()
}

/// Starts the engine and waits for it to load, or `None` if the files are missing.
pub fn motor_listo(nombre: &str) -> Option<(Motor, ConfigMotor)> {
    let config = config(nombre)?;
    let mut motor = Motor::arrancar(&config).unwrap();
    motor
        .esperar_listo(CARGA_MAXIMA, &AtomicBool::new(false))
        .unwrap();
    Some((motor, config))
}
