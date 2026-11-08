//! **Privacidad de los registros (regla 17, ADR 0014)**, con el motor real: se carga un modelo, se le hace una
//! pregunta con una palabra secreta y se comprueba que el registro anota lo que hizo Locall, pero NUNCA la pregunta,
//! la respuesta ni la clave del motor.
//!
//! Es la única prueba de este archivo porque el escritor de registros es global (uno por proceso).

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use locall_config::registros;
use locall_engine::{ConfigMotor, Mensaje, PeticionChat, Rol};
use locall_scheduler::{ArrancadorReal, Carga, Opciones, Planificador};

const SECRETO: &str = "ZANAHORIA-PRIVADA-8314";

#[test]
fn el_registro_no_guarda_conversaciones_ni_claves() {
    let motor = PathBuf::from(r"C:\Locall-poc\motor\cpu\llama-server.exe");
    let modelo = PathBuf::from(r"C:\Locall-poc\modelos\qwen2.5-1.5b-instruct-q4_k_m.gguf");
    if !motor.is_file() || !modelo.is_file() {
        eprintln!("(prueba saltada: faltan el motor o el modelo)");
        return;
    }
    let carpeta = std::env::temp_dir().join(format!("locall-privacidad-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&carpeta);
    registros::iniciar(&carpeta, 7).unwrap();

    let carga = Carga {
        modelo: "privacidad".into(),
        config: ConfigMotor {
            ejecutable: motor,
            modelo,
            contexto: 2048,
            hilos: locall_system::hilos_para_motor(),
            capas_gpu: 0,
            paralelo: 1,
            registro: carpeta.join("motor.log"),
        },
    };
    let plan = Planificador::nuevo(ArrancadorReal::default(), Opciones { inactividad: None });
    let resumen = {
        let prestamo = plan.usar(&carga).unwrap();
        prestamo
            .chat(
                &PeticionChat {
                    mensajes: vec![Mensaje::nuevo(
                        Rol::Usuario,
                        format!("Repite exactamente esta palabra: {SECRETO}"),
                    )],
                    max_tokens: Some(30),
                    temperatura: Some(0.0),
                    semilla: Some(1),
                },
                &AtomicBool::new(false),
                |_| {},
            )
            .unwrap()
    };
    drop(plan);
    log::logger().flush();

    let hoy = locall_system::fecha_hora_local().unwrap().fecha();
    let texto = std::fs::read_to_string(carpeta.join(registros::nombre_del_dia(&hoy))).unwrap();
    eprintln!("── registro ──\n{texto}──────────────");

    // Lo que SÍ debe estar: lo que hizo Locall.
    for esperado in [
        "cargando el modelo «privacidad»",
        "motor arrancado",
        "modelo «privacidad» listo en",
        "respuesta:",
        "modelo «privacidad» descargado (cierre de Locall)",
        "motor parado",
    ] {
        assert!(texto.contains(esperado), "falta «{esperado}»");
    }
    // Lo que NUNCA debe estar: la pregunta, la respuesta ni la clave.
    assert!(
        !texto.contains(SECRETO),
        "¡la pregunta está en el registro!"
    );
    assert!(
        !texto.contains("Repite exactamente"),
        "¡la pregunta está en el registro!"
    );
    let respuesta = resumen.texto.trim();
    if respuesta.len() >= 6 {
        assert!(
            !texto.contains(respuesta),
            "¡la respuesta está en el registro!"
        );
    }
    let hex_seguidos = texto
        .split(|c: char| !c.is_ascii_hexdigit())
        .map(str::len)
        .max()
        .unwrap_or(0);
    assert!(hex_seguidos < 64, "posible clave en el registro");

    let _ = std::fs::remove_dir_all(&carpeta);
}
