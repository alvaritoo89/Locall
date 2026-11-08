//! Pruebas con el `llama-server` real (build b12189) y un modelo pequeño.
//!
//! Si no están los archivos, las pruebas se saltan (con un aviso). Rutas por defecto, cambiables con variables
//! de entorno:
//! - `LOCALL_MOTOR`: `C:\Locall-poc\motor\cpu\llama-server.exe`
//! - `LOCALL_MODELOS`: `C:\Locall-poc\modelos` (usa `qwen2.5-1.5b-instruct-q4_k_m.gguf`)

use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use locall_engine::{ConfigMotor, ErrorMotor, Motor};

const MODELO: &str = "qwen2.5-1.5b-instruct-q4_k_m.gguf";
const CARGA_MAXIMA: Duration = Duration::from_secs(120);

/// Configuración para las pruebas, o `None` si faltan el motor o el modelo.
fn config(nombre: &str) -> Option<ConfigMotor> {
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
fn escucha(puerto: u16) -> bool {
    TcpStream::connect_timeout(&([127, 0, 0, 1], puerto).into(), Duration::from_millis(500)).is_ok()
}

#[test]
fn arranca_carga_y_se_para() {
    let Some(config) = config("ciclo") else {
        return;
    };
    let mut motor = Motor::arrancar(&config).unwrap();
    let tiempo = motor
        .esperar_listo(CARGA_MAXIMA, &AtomicBool::new(false))
        .unwrap();
    eprintln!("modelo cargado en {:.1} s", tiempo.as_secs_f64());

    assert!(motor.sigue_vivo());
    let puerto = motor.puerto();
    assert!(escucha(puerto));
    // Con -lm none el modelo entero está en la RAM del motor (≈1 GB para el 1,5B).
    let ram = locall_system::memoria_de_proceso(motor.pid()).unwrap();
    assert!(ram > 500_000_000, "solo {ram} bytes");

    motor.parar();
    assert!(!escucha(puerto), "el motor sigue escuchando tras pararlo");
    let _ = std::fs::remove_file(&config.registro);
}

#[test]
fn detecta_que_muere_al_cargar_un_modelo_danado() {
    let Some(mut config) = config("danado") else {
        return;
    };
    let falso =
        std::env::temp_dir().join(format!("locall-modelo-falso-{}.gguf", std::process::id()));
    std::fs::write(&falso, b"esto no es un GGUF").unwrap();
    config.modelo = falso.clone();

    let mut motor = Motor::arrancar(&config).unwrap();
    let error = motor
        .esperar_listo(CARGA_MAXIMA, &AtomicBool::new(false))
        .unwrap_err();

    assert!(
        matches!(error, ErrorMotor::TerminoAlCargar { .. }),
        "{error:?}"
    );
    assert!(!motor.sigue_vivo());
    drop(motor);
    let _ = std::fs::remove_file(&falso);
    let _ = std::fs::remove_file(&config.registro);
}

#[test]
fn cancelar_la_carga_para_el_motor() {
    let Some(config) = config("cancelar") else {
        return;
    };
    let mut motor = Motor::arrancar(&config).unwrap();
    let error = motor
        .esperar_listo(CARGA_MAXIMA, &AtomicBool::new(true))
        .unwrap_err();

    assert!(matches!(error, ErrorMotor::Cancelado), "{error:?}");
    assert!(!motor.sigue_vivo());
    drop(motor);
    let _ = std::fs::remove_file(&config.registro);
}

#[test]
fn la_clave_no_aparece_en_el_registro() {
    let Some(config) = config("secreto") else {
        return;
    };
    let mut motor = Motor::arrancar(&config).unwrap();
    motor
        .esperar_listo(CARGA_MAXIMA, &AtomicBool::new(false))
        .unwrap();
    motor.parar();

    let registro = std::fs::read_to_string(&config.registro).unwrap_or_default();
    assert!(
        !registro.is_empty(),
        "el motor debería haber escrito su registro"
    );
    // Nuestra clave son 64 caracteres hexadecimales: no debe haber ninguna secuencia así en el registro.
    let hex_seguidos = registro
        .split(|c: char| !c.is_ascii_hexdigit())
        .map(str::len)
        .max()
        .unwrap_or(0);
    assert!(hex_seguidos < 64, "posible clave en el registro");
    let _ = std::fs::remove_file(&config.registro);
}
