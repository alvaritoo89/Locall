//! El núcleo con archivos reales: importar el modelo de la Fase 0, conversar con el motor de verdad y eliminarlo
//! sin tocar el archivo original. Se salta si faltan el motor o el modelo.

mod comun;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use comun::{CarpetaDePrueba, nucleo};
use locall_core::ErrorNucleo;
use locall_engine::{Evento, Mensaje, PeticionChat, Rol};

fn modelo_local() -> Option<PathBuf> {
    let modelo = PathBuf::from(r"C:\Locall-poc\modelos\qwen2.5-1.5b-instruct-q4_k_m.gguf");
    let motor = PathBuf::from(r"C:\Locall-poc\motor\cpu\llama-server.exe");
    if modelo.is_file() && motor.is_file() {
        Some(modelo)
    } else {
        eprintln!("(prueba saltada: faltan el motor o el modelo)");
        None
    }
}

#[test]
fn importa_conversa_y_elimina_sin_tocar_el_original() {
    let Some(original) = modelo_local() else {
        return;
    };
    let tamano_original = std::fs::metadata(&original).unwrap().len();
    let carpeta = CarpetaDePrueba::nueva("real");
    let nucleo = nucleo(&carpeta, None);

    // 1) Importar: mismo disco → enlace duro, sin copiar 1 GB.
    let modelo = nucleo.importar(&original, Some("qwen-1.5b")).unwrap();
    assert_eq!(modelo.origen, "local");
    assert_eq!(modelo.arquitectura.as_deref(), Some("qwen2"));
    assert_eq!(modelo.cuantizacion.as_deref(), Some("Q4_K_M"));
    assert_eq!(modelo.bytes_totales(), tamano_original);

    // La evaluación «¿cabe?» del 1,5B (≈1,2 GB) en un equipo de 16 GB: holgado o cabe, nunca «no cabe».
    let evaluacion = nucleo.evaluar("qwen-1.5b").unwrap();
    assert!(evaluacion.comprobacion.es_compatible());
    assert!(
        matches!(
            evaluacion.encaje,
            locall_compat::Encaje::Holgado | locall_compat::Encaje::Cabe
        ),
        "{:?}",
        evaluacion.encaje
    );
    assert!(
        nucleo.memoria_del_modelo().is_none(),
        "todavía no hay modelo cargado"
    );

    // 2) Conversar a través del núcleo (carga el motor con los ajustes del usuario).
    let mut texto = String::new();
    let resumen = nucleo
        .chat(
            "qwen-1.5b",
            &PeticionChat {
                mensajes: vec![Mensaje::nuevo(
                    Rol::Usuario,
                    "¿Capital de Francia? Una palabra.",
                )],
                max_tokens: Some(10),
                temperatura: Some(0.0),
                semilla: Some(1),
            },
            &AtomicBool::new(false),
            |e| {
                if let Evento::Texto(t) = e {
                    texto.push_str(t);
                }
            },
        )
        .unwrap();
    eprintln!("respuesta: {texto:?}");
    assert!(texto.contains("Par"), "{texto}");
    assert!(resumen.tokens_generados > 0);
    // Ya cargado: se mide su memoria real (con -lm none, ≈ el tamaño del archivo).
    let memoria = nucleo.memoria_del_modelo().unwrap();
    assert!(memoria > tamano_original / 2, "{memoria}");
    assert!(
        nucleo
            .modelo("qwen-1.5b")
            .unwrap()
            .unwrap()
            .ultimo_uso
            .is_some()
    );

    // 3) Eliminar: se descarga de la memoria y desaparece de Locall…
    nucleo.eliminar("qwen-1.5b").unwrap();
    assert!(nucleo.modelos().unwrap().is_empty());
    assert!(matches!(
        nucleo.chat(
            "qwen-1.5b",
            &PeticionChat::default(),
            &AtomicBool::new(false),
            |_| {}
        ),
        Err(ErrorNucleo::NoInstalado(_))
    ));
    // …pero el archivo original del usuario sigue ahí, entero.
    assert_eq!(std::fs::metadata(&original).unwrap().len(), tamano_original);
}

#[test]
#[ignore = "usa internet (Hugging Face)"]
fn instala_de_hugging_face_de_verdad() {
    let carpeta = CarpetaDePrueba::nueva("hf");
    let nucleo = nucleo(&carpeta, None);

    let (info, ggufs) = nucleo.consultar("ggml-org/tiny-llamas", "main").unwrap();
    assert_eq!(info.commit.len(), 40);
    assert!(ggufs.iter().any(|a| a.ruta == "stories260K.gguf"));

    let modelo = nucleo
        .instalar(
            &locall_core::PeticionInstalar {
                repo: "ggml-org/tiny-llamas".into(),
                revision: "main".into(),
                archivo: "stories260K.gguf".into(),
                id: None,
            },
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
    assert_eq!(modelo.id.to_string(), "stories260k");
    assert_eq!(modelo.revision, info.commit);
    assert_eq!(
        modelo.archivos[0].huella.hex(),
        "047bf46455a544931cff6fef14d7910154c56afbc23ab1c5e56a72e69912c04b"
    );
    assert_eq!(modelo.arquitectura.as_deref(), Some("llama"));
}

#[test]
fn las_conversaciones_se_guardan_y_se_envian_enteras() {
    let Some(original) = modelo_local() else {
        return;
    };
    let carpeta = CarpetaDePrueba::nueva("conversacion");
    let nucleo = nucleo(&carpeta, None);
    nucleo.importar(&original, Some("qwen")).unwrap();
    let c = nucleo.crear_conversacion("Prueba de memoria").unwrap();
    let nada = AtomicBool::new(false);

    let primera = nucleo
        .conversar(
            c.id,
            "Me llamo Ana. Responde solo: vale.",
            "qwen",
            "Responde en español.",
            Some(0.0),
            &nada,
            |_| {},
        )
        .unwrap();
    assert!(primera.respuesta.is_some());
    // La segunda pregunta solo se puede contestar si el modelo recibe la conversación guardada.
    let segunda = nucleo
        .conversar(
            c.id,
            "¿Cómo me llamo? Responde con una palabra.",
            "qwen",
            "Responde en español.",
            Some(0.0),
            &nada,
            |_| {},
        )
        .unwrap();
    let texto = segunda.respuesta.as_ref().unwrap().contenido.clone();
    eprintln!("respuesta: {texto:?}");
    assert!(texto.contains("Ana"), "{texto}");

    let (conversacion, mensajes) = nucleo.conversacion(c.id).unwrap();
    assert_eq!(conversacion.mensajes, 4);
    assert_eq!(conversacion.modelo.as_deref(), Some("qwen"));
    assert_eq!(conversacion.sistema, "Responde en español.");
    assert!(mensajes[3].tokens.unwrap() > 0 && mensajes[3].velocidad.unwrap() > 0.0);

    // Un error del modelo (aquí, un modelo que no existe) no deja la pregunta guardada.
    assert!(
        nucleo
            .conversar(c.id, "¿Sigues ahí?", "no-existe", "", None, &nada, |_| {})
            .is_err()
    );
    let (tras_error, _) = nucleo.conversacion(c.id).unwrap();
    assert_eq!(tras_error.mensajes, 4, "no se guardó la pregunta");
    assert_eq!(
        tras_error.modelo.as_deref(),
        Some("qwen"),
        "ni el modelo que no existe"
    );

    nucleo.eliminar_conversacion(c.id).unwrap();
    assert!(matches!(
        nucleo.conversacion(c.id),
        Err(ErrorNucleo::ConversacionNoEncontrada(_))
    ));
    assert!(nucleo.conversaciones().unwrap().is_empty());
}

#[test]
fn parar_y_enviar_enseguida_no_desordena_los_mensajes() {
    let Some(original) = modelo_local() else {
        return;
    };
    let carpeta = CarpetaDePrueba::nueva("turno");
    let nucleo = nucleo(&carpeta, None);
    nucleo.importar(&original, Some("qwen")).unwrap();
    let c = nucleo.crear_conversacion("Turnos").unwrap();
    let parar = AtomicBool::new(false);
    let nada = AtomicBool::new(false);

    std::thread::scope(|hilos| {
        // Una respuesta larga, que se para al llegar el primer texto…
        let larga = hilos.spawn(|| {
            nucleo.conversar(
                c.id,
                "Cuenta del 1 al 200, separados por comas.",
                "qwen",
                "",
                Some(0.0),
                &parar,
                |e| {
                    if matches!(e, Evento::Texto(_)) {
                        parar.store(true, Ordering::Relaxed);
                    }
                },
            )
        });
        // …y, mientras tanto, otra pregunta en la misma conversación: debe esperar su turno.
        while !parar.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let corta = nucleo
            .conversar(c.id, "Di solo: hola.", "qwen", "", Some(0.0), &nada, |_| {})
            .unwrap();
        let larga = larga.join().unwrap().unwrap();
        assert!(larga.respuesta.as_ref().unwrap().cortada);
        assert!(corta.respuesta.is_some());
    });

    let (_, mensajes) = nucleo.conversacion(c.id).unwrap();
    let orden: Vec<_> = mensajes.iter().map(|m| (m.autor, m.cortada)).collect();
    use locall_core::Autor::{Asistente, Usuario};
    assert_eq!(
        orden,
        [
            (Usuario, false),
            (Asistente, true),
            (Usuario, false),
            (Asistente, false)
        ]
    );
    assert!(mensajes[0].contenido.starts_with("Cuenta"));
    assert!(mensajes[2].contenido.starts_with("Di solo"));
}

#[test]
#[ignore = "usa internet (Hugging Face)"]
fn busca_en_hugging_face_de_verdad() {
    let carpeta = CarpetaDePrueba::nueva("hf-buscar");
    let nucleo = nucleo(&carpeta, None);
    let encontrados = nucleo.buscar("tiny-llamas", 5).unwrap();
    eprintln!("{encontrados:#?}");
    assert!(encontrados.iter().any(|r| r.repo == "ggml-org/tiny-llamas"));
    assert!(encontrados.len() <= 5);
}
