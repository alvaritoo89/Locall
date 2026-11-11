//! Tests of the pre-load check that need internal functions.

use std::path::{Path, PathBuf};

use locall_compat::Encaje;
use locall_engine::ConfigMotor;

use crate::Carga;
use crate::comprobacion::{ErrorCarga, archivos_del_modelo, decidir, evaluar_con_memoria};

const GB: u64 = 1_000_000_000;

fn carga_de(modelo: &Path, contexto: u32) -> Carga {
    Carga {
        modelo: "prueba".into(),
        config: ConfigMotor {
            ejecutable: PathBuf::from("llama-server.exe"),
            modelo: modelo.to_path_buf(),
            contexto,
            hilos: 1,
            capas_gpu: 0,
            paralelo: 1,
            registro: PathBuf::from("motor.log"),
        },
    }
}

/// The Phase 0 1.5B model, or `None` if it is not there.
fn modelo_pequeno() -> Option<PathBuf> {
    let ruta = std::env::var_os("LOCALL_MODELOS")
        .map_or_else(|| PathBuf::from(r"C:\Locall-poc\modelos"), PathBuf::from)
        .join("qwen2.5-1.5b-instruct-q4_k_m.gguf");
    if ruta.is_file() {
        Some(ruta)
    } else {
        eprintln!("(prueba saltada: falta {})", ruta.display());
        None
    }
}

#[test]
fn deduce_las_partes_de_un_modelo_dividido() {
    let principal = Path::new(r"C:\m\qwen-7b-q4-00001-of-00003.gguf");
    assert_eq!(
        archivos_del_modelo(principal, Some(3)),
        [
            PathBuf::from(r"C:\m\qwen-7b-q4-00001-of-00003.gguf"),
            PathBuf::from(r"C:\m\qwen-7b-q4-00002-of-00003.gguf"),
            PathBuf::from(r"C:\m\qwen-7b-q4-00003-of-00003.gguf"),
        ]
    );
    // Not split, or with a name that does not follow the format: only the main one.
    let solo = Path::new(r"C:\m\qwen.gguf");
    assert_eq!(archivos_del_modelo(solo, None), [solo.to_path_buf()]);
    assert_eq!(archivos_del_modelo(solo, Some(1)), [solo.to_path_buf()]);
    assert_eq!(archivos_del_modelo(solo, Some(3)), [solo.to_path_buf()]);
}

#[test]
fn no_deja_cargar_si_no_cabe() {
    let Some(modelo) = modelo_pequeno() else {
        return;
    };
    let carga = carga_de(&modelo, 4096);

    // With 1 GB free, the 1.5B (≈1.2 GB) does not fit.
    let evaluacion = evaluar_con_memoria(&carga, GB).unwrap();
    assert_eq!(evaluacion.encaje, Encaje::NoCabe);
    let error = decidir(&evaluacion, GB).unwrap_err();
    let ErrorCarga::NoCabe {
        necesarios,
        disponibles,
    } = error
    else {
        panic!("{error:?}")
    };
    assert!(necesarios > GB, "{necesarios}");
    assert_eq!(disponibles, GB);
    let mensaje = error_no_cabe(necesarios).to_string();
    assert!(mensaje.contains("1.0 GB libres"), "{mensaje}");

    // With 8 GB free, it does.
    let evaluacion = evaluar_con_memoria(&carga, 8 * GB).unwrap();
    assert!(decidir(&evaluacion, 8 * GB).is_ok());
}

fn error_no_cabe(necesarios: u64) -> ErrorCarga {
    ErrorCarga::NoCabe {
        necesarios,
        disponibles: GB,
    }
}

#[test]
fn justo_si_deja_cargar() {
    let Some(modelo) = modelo_pequeno() else {
        return;
    };
    let carga = carga_de(&modelo, 4096);
    // ≈1.3 GB needed with 4 GB free: below the 4 GB margin threshold → "tight", but allowed (D7).
    let evaluacion = evaluar_con_memoria(&carga, 4 * GB).unwrap();
    assert_eq!(evaluacion.encaje, Encaje::Justo);
    assert!(decidir(&evaluacion, 4 * GB).is_ok());
}

#[test]
fn los_mensajes_no_muestran_rutas() {
    let error = ErrorCarga::FaltaParte(PathBuf::from(
        r"C:\Users\ana\modelos\qwen-00002-of-00002.gguf",
    ));
    let texto = error.to_string();
    assert!(texto.contains("qwen-00002-of-00002.gguf"));
    assert!(!texto.contains("ana"), "{texto}");
}
