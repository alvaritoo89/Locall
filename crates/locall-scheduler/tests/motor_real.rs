//! Scheduler test with the real `llama-server` (build b12189): loads a model, chats, switches to another one and
//! checks that the first one really stopped. Then, idle unloading.
//!
//! Skipped if the files are missing. Default paths (changeable with `LOCALL_MOTOR` and `LOCALL_MODELOS`):
//! `C:\Locall-poc\motor\cpu\llama-server.exe` and `C:\Locall-poc\modelos` (Qwen 2.5 1.5B and Qwen3 4B).

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
            paralelo: 1,
            registro: std::env::temp_dir()
                .join(format!("locall-plan-{nombre}-{}.log", std::process::id())),
        },
    })
}

/// Does the process still exist? (If it already finished, Windows does not allow querying its memory.)
fn proceso_vivo(pid: u32) -> bool {
    locall_system::memoria_de_proceso(pid).is_ok()
}

/// Waits up to 3 s for the process to disappear.
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

    // 1) Load the small one and chat with it through the loan.
    let prestamo = plan.usar(&pequeno).unwrap();
    let pid_pequeno = prestamo.pid();
    let resumen = prestamo
        .chat(&pregunta(), &AtomicBool::new(false), |_| {})
        .unwrap();
    assert!(resumen.tokens_generados > 0);
    drop(prestamo);

    // 2) Request it again: the same process (it is not reloaded).
    assert_eq!(plan.usar(&pequeno).unwrap().pid(), pid_pequeno);

    // 3) Switch to the medium one: the small one must stop.
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

    // 4) When the scheduler is dropped, everything stops.
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

#[test]
fn dos_conversaciones_a_la_vez_con_un_solo_hueco() {
    let Some(pequeno) = carga("dos-a-la-vez", "qwen2.5-1.5b-instruct-q4_k_m.gguf") else {
        return;
    };
    let plan = Planificador::nuevo(ArrancadorReal::default(), Opciones { inactividad: None });
    // With parallel = 1, the engine serves one and the other waits inside the engine: both must finish fine.
    let resumenes: Vec<_> = thread::scope(|s| {
        let hilos: Vec<_> = (0..2)
            .map(|_| {
                s.spawn(|| {
                    let prestamo = plan.usar(&pequeno).unwrap();
                    prestamo
                        .chat(&pregunta(), &AtomicBool::new(false), |_| {})
                        .unwrap()
                })
            })
            .collect();
        hilos.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert!(resumenes.iter().all(|r| r.tokens_generados > 0));
    drop(plan);
    let _ = std::fs::remove_file(&pequeno.config.registro);
}

#[test]
fn si_el_motor_real_se_cierra_solo_se_recupera() {
    let Some(pequeno) = carga("se-cierra", "qwen2.5-1.5b-instruct-q4_k_m.gguf") else {
        return;
    };
    let plan = Planificador::nuevo(ArrancadorReal::default(), Opciones { inactividad: None });
    let pid = plan.usar(&pequeno).unwrap().pid();

    // Fake the engine exiting by itself (as if Windows closed it for lack of memory): the test terminates the engine
    // process it started itself.
    let cerrado = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .output()
        .unwrap();
    assert!(
        cerrado.status.success(),
        "no se pudo cerrar el motor de la prueba"
    );
    assert!(espera_a_que_muera(pid));

    // The watcher removes it: the state stops saying «ready» (it checks every 2 s).
    let limite = Instant::now() + Duration::from_secs(10);
    while plan.estado() != Estado::Vacio && Instant::now() < limite {
        thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(
        plan.estado(),
        Estado::Vacio,
        "el motor muerto sigue como «listo»"
    );

    // And when it is requested again, another engine that answers is loaded.
    let prestamo = plan.usar(&pequeno).unwrap();
    assert_ne!(prestamo.pid(), pid, "es un motor nuevo");
    let resumen = prestamo
        .chat(&pregunta(), &AtomicBool::new(false), |_| {})
        .unwrap();
    assert!(resumen.tokens_generados > 0);
    drop(prestamo);
    drop(plan);
    let _ = std::fs::remove_file(&pequeno.config.registro);
}
