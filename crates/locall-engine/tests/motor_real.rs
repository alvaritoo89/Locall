//! Lifecycle tests with the real `llama-server` (build b12189) and a small model (see `comun`).

mod comun;

use std::sync::atomic::AtomicBool;

use comun::{CARGA_MAXIMA, config, escucha};
use locall_engine::{Causa, ConfigMotor, ErrorMotor, Motor};

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
    // With -lm none the whole model is in the engine's RAM (≈1 GB for the 1.5B).
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

    assert_eq!(error.causa(), Causa::ModeloDanado, "{error:?}");
    eprintln!("mensaje: {error}");
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
    // Our key is 64 hexadecimal characters: there must be no such sequence in the log.
    let hex_seguidos = registro
        .split(|c: char| !c.is_ascii_hexdigit())
        .map(str::len)
        .max()
        .unwrap_or(0);
    assert!(hex_seguidos < 64, "posible clave en el registro");
    let _ = std::fs::remove_file(&config.registro);
}

/// Starts with `config` and returns the load error.
fn error_al_cargar(config: &ConfigMotor) -> ErrorMotor {
    match Motor::arrancar_listo(config, CARGA_MAXIMA, &AtomicBool::new(false)) {
        Ok(_) => panic!("debería haber fallado"),
        Err(e) => e,
    }
}

#[test]
fn reconoce_una_arquitectura_no_soportada() {
    let Some(mut config) = config("arquitectura") else {
        return;
    };
    // A minimal GGUF v3: no tensors and a made-up architecture.
    let mut gguf = b"GGUF".to_vec();
    gguf.extend(3u32.to_le_bytes()); // version
    gguf.extend(0u64.to_le_bytes()); // tensors
    gguf.extend(1u64.to_le_bytes()); // metadata
    let cadena = |v: &mut Vec<u8>, s: &str| {
        v.extend((s.len() as u64).to_le_bytes());
        v.extend(s.as_bytes());
    };
    cadena(&mut gguf, "general.architecture");
    gguf.extend(8u32.to_le_bytes()); // type: string
    cadena(&mut gguf, "inventada");
    let ruta = std::env::temp_dir().join(format!("locall-arq-{}.gguf", std::process::id()));
    std::fs::write(&ruta, gguf).unwrap();
    config.modelo = ruta.clone();

    let error = error_al_cargar(&config);
    eprintln!("mensaje: {error}");
    assert_eq!(
        error.causa(),
        Causa::ArquitecturaNoSoportada("inventada".into())
    );
    let _ = std::fs::remove_file(&ruta);
    let _ = std::fs::remove_file(&config.registro);
}

#[test]
fn reconoce_la_falta_de_memoria() {
    let Some(mut config) = config("memoria") else {
        return;
    };
    config.contexto = 4_000_000; // the KV cache would need more than 100 GB
    let error = error_al_cargar(&config);
    eprintln!("mensaje: {error}");
    assert_eq!(error.causa(), Causa::MemoriaInsuficiente);
    assert!(
        !error.to_string().contains("Locall-poc"),
        "sin rutas en el mensaje"
    );
    let _ = std::fs::remove_file(&config.registro);
}
