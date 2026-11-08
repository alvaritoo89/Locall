//! Ayudantes compartidos por las pruebas con el motor real.
//!
//! Si no están los archivos, las pruebas se saltan (con un aviso). Rutas por defecto, cambiables con variables
//! de entorno:
//! - `LOCALL_MOTOR`: `C:\Locall-poc\motor\cpu\llama-server.exe`
//! - `LOCALL_MODELOS`: `C:\Locall-poc\modelos` (usa `qwen2.5-1.5b-instruct-q4_k_m.gguf`)

#![allow(dead_code)] // cada archivo de pruebas usa solo parte de estos ayudantes

use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use locall_engine::{ConfigMotor, Motor};

pub const MODELO: &str = "qwen2.5-1.5b-instruct-q4_k_m.gguf";
pub const CARGA_MAXIMA: Duration = Duration::from_secs(120);

/// Configuración para las pruebas, o `None` si faltan el motor o el modelo.
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
        registro: std::env::temp_dir()
            .join(format!("locall-motor-{nombre}-{}.log", std::process::id())),
    })
}

/// ¿Hay algo escuchando en ese puerto?
pub fn escucha(puerto: u16) -> bool {
    TcpStream::connect_timeout(&([127, 0, 0, 1], puerto).into(), Duration::from_millis(500)).is_ok()
}

/// Arranca el motor y espera a que cargue, o `None` si faltan los archivos.
pub fn motor_listo(nombre: &str) -> Option<(Motor, ConfigMotor)> {
    let config = config(nombre)?;
    let mut motor = Motor::arrancar(&config).unwrap();
    motor
        .esperar_listo(CARGA_MAXIMA, &AtomicBool::new(false))
        .unwrap();
    Some((motor, config))
}
