//! Pruebas que no necesitan el motor real.

use std::path::PathBuf;

use super::*;

fn config() -> ConfigMotor {
    ConfigMotor {
        ejecutable: PathBuf::from(r"C:\no-existe\llama-server.exe"),
        modelo: PathBuf::from(r"C:\modelos\m.gguf"),
        contexto: 4096,
        hilos: 1,
        capas_gpu: 0,
        registro: std::env::temp_dir().join("locall-motor-prueba.log"),
    }
}

#[test]
fn argumentos_seguros() {
    let args: Vec<String> = argumentos(&config(), 5555)
        .into_iter()
        .map(|a| a.into_string().unwrap())
        .collect();
    let valor = |nombre: &str| {
        let i = args.iter().position(|a| a == nombre).unwrap();
        args[i + 1].as_str()
    };
    assert_eq!(valor("--host"), "127.0.0.1");
    assert_eq!(valor("--port"), "5555");
    assert_eq!(valor("--load-mode"), "none");
    assert_eq!(valor("--cors-origins"), "localhost");
    assert_eq!(valor("--ctx-size"), "4096");
    assert!(args.iter().any(|a| a == "--no-webui"));
    // La clave jamás va en los argumentos.
    assert!(
        !args
            .iter()
            .any(|a| a.contains("api-key") || a.contains("api_key"))
    );
}

#[test]
fn la_clave_es_aleatoria_y_larga() {
    let a = generar_clave().unwrap();
    let b = generar_clave().unwrap();
    assert_eq!(a.len(), 64);
    assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(a, b);
}

#[test]
fn falla_si_no_existe_el_ejecutable() {
    let error = Motor::arrancar(&config()).unwrap_err();
    assert!(matches!(error, ErrorMotor::NoEncontrado(_)), "{error:?}");
}

#[test]
fn rechaza_hilos_imposibles() {
    // Un "ejecutable" y un "modelo" que existen (este mismo programa de pruebas) para llegar a la validación.
    let este = std::env::current_exe().unwrap();
    let mut config = config();
    config.ejecutable = este.clone();
    config.modelo = este;
    config.hilos = 100_000;
    let error = Motor::arrancar(&config).unwrap_err();
    assert!(matches!(error, ErrorMotor::Hilos(_)), "{error:?}");
}

#[test]
fn el_debug_no_muestra_la_clave() {
    // Un proceso cualquiera que termina enseguida, solo para construir un `Motor`.
    let proceso = std::process::Command::new("cmd")
        .args(["/C", "exit"])
        .spawn()
        .unwrap();
    let motor = Motor {
        proceso,
        puerto: 1,
        clave: "SECRETO-DE-PRUEBA".into(),
        registro: PathBuf::from("r.log"),
        agente: ureq::Agent::new_with_defaults(),
        _trabajo: Trabajo::nuevo().unwrap(),
    };
    assert!(!format!("{motor:?}").contains("SECRETO-DE-PRUEBA"));
}
