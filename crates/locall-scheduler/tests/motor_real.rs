//! Prueba del planificador con el `llama-server` real (build b12189): carga un modelo, conversa, cambia a otro y
//! comprueba que el primero se ha parado de verdad. Después, la descarga por inactividad.
//!
//! Se salta si faltan los archivos. Rutas por defecto (cambiables con `LOCALL_MOTOR` y `LOCALL_MODELOS`):
//! `C:\Locall-poc\motor\cpu\llama-server.exe` y `C:\Locall-poc\modelos` (Qwen 2.5 1.5B y Qwen3 4B).

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::thread;
use std::time::{Duration, Instant};

use locall_engine::{ConfigMotor, Mensaje, PeticionChat, Rol};
use locall_scheduler::{ArrancadorReal, Carga, Estado, Opciones, Planificador};

fn carga(nombre: &str, archivo: &str) -> Option<Carga> {
    let motor = std::env::var_os("LOCALL_MOTOR").map_or_else(
        || PathBuf::from(r"C:\Locall-poc\motor\cpu\llama-server.exe"),
        PathBuf::from,
    );
    let modelos = std::env::var_os("LOCALL_MODELOS")
        .map_or_else(|| PathBuf::from(r"C:\Locall-poc\modelos"), PathBuf::from);
    let modelo = modelos.join(archivo);
    if !motor.is_file() || !modelo.is_file() {
        eprintln!(
            "(prueba saltada: faltan {} o {})",
            motor.display(),
            modelo.display()
        );
        return None;
    }
    Some(Carga {
        modelo: nombre.into(),
        config: ConfigMotor {
            ejecutable: motor,
            modelo,
            contexto: 2048,
            hilos: locall_system::hilos_para_motor(),
            capas_gpu: 0,
            registro: std::env::temp_dir()
                .join(format!("locall-plan-{nombre}-{}.log", std::process::id())),
        },
    })
}

/// ¿Sigue existiendo el proceso? (Si ya terminó, Windows no deja consultar su memoria.)
fn proceso_vivo(pid: u32) -> bool {
    locall_system::memoria_de_proceso(pid).is_ok()
}

/// Espera hasta 3 s a que el proceso desaparezca.
fn espera_a_que_muera(pid: u32) -> bool {
    let inicio = Instant::now();
    while inicio.elapsed() < Duration::from_secs(3) {
        if !proceso_vivo(pid) {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    false
}

fn pregunta() -> PeticionChat {
    PeticionChat {
        mensajes: vec![Mensaje::nuevo(Rol::Usuario, "Di solo: hola")],
        max_tokens: Some(5),
        temperatura: Some(0.0),
        semilla: Some(1),
    }
}

#[test]
fn carga_conversa_y_cambia_de_modelo() {
    let (Some(pequeno), Some(mediano)) = (
        carga("qwen2.5-1.5b", "qwen2.5-1.5b-instruct-q4_k_m.gguf"),
        carga("qwen3-4b", "Qwen3-4B-Q4_K_M.gguf"),
    ) else {
        return;
    };
    let plan = Planificador::nuevo(ArrancadorReal::default(), Opciones { inactividad: None });

    // 1) Cargar el pequeño y conversar con él a través del préstamo.
    let prestamo = plan.usar(&pequeno).unwrap();
    let pid_pequeno = prestamo.pid();
    let resumen = prestamo
        .chat(&pregunta(), &AtomicBool::new(false), |_| {})
        .unwrap();
    assert!(resumen.tokens_generados > 0);
    drop(prestamo);

    // 2) Volver a pedirlo: el mismo proceso (no se recarga).
    assert_eq!(plan.usar(&pequeno).unwrap().pid(), pid_pequeno);

    // 3) Cambiar al mediano: el pequeño debe pararse.
    let prestamo = plan.usar(&mediano).unwrap();
    assert_ne!(prestamo.pid(), pid_pequeno);
    assert!(
        espera_a_que_muera(pid_pequeno),
        "el modelo anterior sigue en marcha"
    );
    assert_eq!(
        plan.estado(),
        Estado::Listo {
            modelo: "qwen3-4b".into(),
            en_uso: 1
        }
    );
    let pid_mediano = prestamo.pid();
    drop(prestamo);

    // 4) Al soltar el planificador, se para todo.
    drop(plan);
    assert!(
        espera_a_que_muera(pid_mediano),
        "el motor sigue en marcha tras cerrar"
    );
    let _ = std::fs::remove_file(&pequeno.config.registro);
    let _ = std::fs::remove_file(&mediano.config.registro);
}

#[test]
fn descarga_el_modelo_real_por_inactividad() {
    let Some(pequeno) = carga("inactivo", "qwen2.5-1.5b-instruct-q4_k_m.gguf") else {
        return;
    };
    let plan = Planificador::nuevo(
        ArrancadorReal::default(),
        Opciones {
            inactividad: Some(Duration::from_secs(1)),
        },
    );
    let pid = plan.usar(&pequeno).unwrap().pid();
    assert!(proceso_vivo(pid));

    thread::sleep(Duration::from_millis(1500));
    assert!(espera_a_que_muera(pid), "no se descargó por inactividad");
    assert_eq!(plan.estado(), Estado::Vacio);
    let _ = std::fs::remove_file(&pequeno.config.registro);
}
