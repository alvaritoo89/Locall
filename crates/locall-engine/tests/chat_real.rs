//! Pruebas de conversación con el `llama-server` real (ver `comun`).

mod comun;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use comun::motor_listo;
use locall_engine::{Causa, ErrorMotor, Evento, Fin, Mensaje, PeticionChat, Rol};

fn pregunta(texto: &str, max_tokens: u32) -> PeticionChat {
    PeticionChat {
        mensajes: vec![
            Mensaje::nuevo(
                Rol::Sistema,
                "Eres un asistente breve. Respondes en español.",
            ),
            Mensaje::nuevo(Rol::Usuario, texto),
        ],
        max_tokens: Some(max_tokens),
        temperatura: Some(0.0),
        semilla: Some(1),
    }
}

#[test]
fn responde_por_trozos() {
    let Some((motor, config)) = motor_listo("chat") else {
        return;
    };
    let mut trozos = 0;
    let mut leyendo = false;
    let mut recibido = String::new();

    let resumen = motor
        .chat(
            &pregunta("¿Cuál es la capital de Francia? Una palabra.", 30),
            &AtomicBool::new(false),
            |evento| match evento {
                Evento::Texto(t) => {
                    trozos += 1;
                    recibido.push_str(t);
                }
                Evento::Leyendo { .. } => leyendo = true,
                Evento::Razonamiento(_) => {}
            },
        )
        .unwrap();

    eprintln!("respuesta: {:?} ({resumen:?})", resumen.texto);
    assert!(resumen.texto.contains("París") || resumen.texto.contains("Paris"));
    assert_eq!(
        resumen.texto, recibido,
        "el resumen es la suma de los trozos"
    );
    assert!(trozos >= 1);
    assert!(leyendo, "debería avisar mientras lee el prompt");
    assert_eq!(resumen.fin, Fin::Completo);
    assert!(resumen.tokens_leidos > 0 && resumen.tokens_generados > 0);
    assert!(resumen.escritura_por_segundo.unwrap() > 0.0);
    drop(motor);
    let _ = std::fs::remove_file(&config.registro);
}

#[test]
fn respeta_el_maximo_de_tokens() {
    let Some((motor, config)) = motor_listo("limite") else {
        return;
    };
    let resumen = motor
        .chat(
            &pregunta("Escribe una redacción larga sobre el mar.", 5),
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
    assert_eq!(resumen.fin, Fin::Limite);
    assert_eq!(resumen.tokens_generados, 5);
    drop(motor);
    let _ = std::fs::remove_file(&config.registro);
}

#[test]
fn cancelar_detiene_la_generacion() {
    let Some((motor, config)) = motor_listo("cancelar-chat") else {
        return;
    };
    let cancelar = AtomicBool::new(false);
    let mut trozos = 0;

    let resumen = motor
        .chat(
            &pregunta("Escribe una redacción muy larga sobre el mar.", 400),
            &cancelar,
            |evento| {
                if let Evento::Texto(_) = evento {
                    trozos += 1;
                    if trozos == 5 {
                        cancelar.store(true, Ordering::Relaxed);
                    }
                }
            },
        )
        .unwrap();

    assert_eq!(resumen.fin, Fin::Cancelado);
    assert!(
        !resumen.texto.is_empty(),
        "conserva lo generado hasta cancelar"
    );
    // El motor debe quedar libre enseguida (400 tokens habrían tardado más de 10 s).
    let inicio = Instant::now();
    while motor.ocupado().unwrap() {
        assert!(
            inicio.elapsed() < Duration::from_secs(2),
            "el motor sigue generando tras cancelar"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    drop(motor);
    let _ = std::fs::remove_file(&config.registro);
}

#[test]
fn informa_si_el_prompt_no_cabe() {
    let Some((motor, config)) = motor_listo("contexto") else {
        return;
    };
    let enorme = "hola ".repeat(3000); // ≈ 3000 tokens en un contexto de 2048
    let error = motor
        .chat(&pregunta(&enorme, 10), &AtomicBool::new(false), |_| {})
        .unwrap_err();

    eprintln!("mensaje: {error}");
    assert!(matches!(error, ErrorMotor::Respuesta { estado: 400, .. }));
    match error.causa() {
        Causa::ContextoExcedido {
            pedidos: Some(p),
            maximo: Some(2048),
        } => assert!(p > 2048),
        otra => panic!("causa inesperada: {otra:?}"),
    }
    drop(motor);
    let _ = std::fs::remove_file(&config.registro);
}
