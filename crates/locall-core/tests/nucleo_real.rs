//! El núcleo con archivos reales: importar el modelo de la Fase 0, conversar con el motor de verdad y eliminarlo
//! sin tocar el archivo original. Se salta si faltan el motor o el modelo.

mod comun;

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

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
