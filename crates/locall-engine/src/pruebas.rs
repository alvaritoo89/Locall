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

// ─── Conversación (sin motor) ────────────────────────────────────────────────

use crate::chat::{Tiempos, Trozo, cuerpo, error_de_respuesta, interpretar};
use crate::{Fin, Mensaje, PeticionChat, Rol};

// Líneas reales de llama-server b12189 (acortadas). Fíjate en "model": lleva la ruta completa del modelo.
const PROGRESO: &str = r#"{"choices":[{"finish_reason":null,"index":0,"delta":{"role":"assistant","content":null}}],"model":"C:/Locall-poc/modelos/qwen.gguf","object":"chat.completion.chunk","prompt_progress":{"total":32,"cache":0,"processed":32,"time_ms":178}}"#;
const TEXTO: &str = r#"{"choices":[{"finish_reason":null,"index":0,"delta":{"content":"Hola"}}],"model":"C:/Locall-poc/modelos/qwen.gguf","object":"chat.completion.chunk"}"#;
const FINAL: &str = r#"{"choices":[{"finish_reason":"length","index":0,"delta":{}}],"model":"C:/Locall-poc/modelos/qwen.gguf","object":"chat.completion.chunk","timings":{"cache_n":0,"prompt_n":32,"prompt_ms":180.8,"prompt_per_second":176.9,"predicted_n":8,"predicted_ms":201.0,"predicted_per_second":34.8}}"#;

#[test]
fn interpreta_el_progreso() {
    let trozo = interpretar(PROGRESO).unwrap();
    assert_eq!(trozo.progreso, Some((32, 32)));
    assert_eq!(trozo.texto, None, "content: null no es texto");
}

#[test]
fn interpreta_el_texto_y_el_razonamiento() {
    assert_eq!(interpretar(TEXTO).unwrap().texto.as_deref(), Some("Hola"));
    let pensando = r#"{"choices":[{"finish_reason":null,"delta":{"reasoning_content":"Veamos"}}]}"#;
    assert_eq!(
        interpretar(pensando).unwrap().razonamiento.as_deref(),
        Some("Veamos")
    );
}

#[test]
fn interpreta_el_final_y_los_tiempos() {
    let trozo = interpretar(FINAL).unwrap();
    assert_eq!(trozo.fin, Some(Fin::Limite));
    assert_eq!(
        trozo,
        Trozo {
            fin: Some(Fin::Limite),
            tiempos: Some(Tiempos {
                leidos: 32,
                cache: 0,
                generados: 8,
                lectura: Some(176.9),
                escritura: Some(34.8),
            }),
            ..Trozo::default()
        }
    );
    // Nada de lo que devolvemos lleva la ruta del modelo (D4).
    assert!(!format!("{trozo:?}").contains("Locall-poc"));
}

#[test]
fn el_progreso_descuenta_la_cache() {
    let datos =
        r#"{"choices":[{"delta":{}}],"prompt_progress":{"total":100,"cache":60,"processed":80}}"#;
    assert_eq!(interpretar(datos).unwrap().progreso, Some((20, 40)));
}

#[test]
fn un_error_a_mitad_de_respuesta_es_un_error() {
    let datos = r#"{"error":{"code":500,"message":"fallo","type":"server_error"}}"#;
    assert!(matches!(
        interpretar(datos),
        Err(ErrorMotor::Respuesta { estado: 500, .. })
    ));
    assert!(matches!(
        interpretar("no es json"),
        Err(ErrorMotor::Protocolo(_))
    ));
}

#[test]
fn traduce_los_errores_del_motor() {
    let texto = r#"{"error":{"code":400,"message":"request (6030 tokens) exceeds the available context size (2048 tokens), try increasing it","type":"exceed_context_size_error","n_prompt_tokens":6030,"n_ctx":2048}}"#;
    let ErrorMotor::Respuesta {
        estado,
        tipo,
        mensaje,
    } = error_de_respuesta(400, texto)
    else {
        panic!()
    };
    assert_eq!((estado, tipo.as_str()), (400, "exceed_context_size_error"));
    assert!(mensaje.contains("exceeds"));
    // Una respuesta que no es JSON también se convierte en error (con el estado HTTP).
    assert!(matches!(
        error_de_respuesta(502, "<html>"),
        ErrorMotor::Respuesta { estado: 502, .. }
    ));
}

#[test]
fn construye_el_cuerpo_de_la_peticion() {
    let peticion = PeticionChat {
        mensajes: vec![
            Mensaje::nuevo(Rol::Sistema, "breve"),
            Mensaje::nuevo(Rol::Usuario, "hola"),
        ],
        max_tokens: Some(10),
        temperatura: None,
        semilla: Some(7),
    };
    let json = cuerpo(&peticion);
    assert_eq!(json["messages"][0]["role"], "system");
    assert_eq!(json["messages"][1]["content"], "hola");
    assert_eq!(json["stream"], true);
    assert_eq!(json["return_progress"], true);
    assert_eq!(json["max_tokens"], 10);
    assert_eq!(json["seed"], 7);
    assert!(
        json.get("temperature").is_none(),
        "sin temperatura, la del modelo"
    );
}
