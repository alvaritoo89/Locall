//! La comprobación antes de cargar, con modelos reales y archivos preparados a propósito. Ninguna de estas pruebas
//! debe llegar a arrancar el motor: los fallos se detectan antes, en un instante.
//!
//! Se salta si faltan los archivos (rutas por defecto: `C:\Locall-poc\motor\cpu` y `C:\Locall-poc\modelos`).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use locall_compat::Encaje;
use locall_engine::ConfigMotor;
use locall_scheduler::{
    Arrancador, ArrancadorReal, Carga, ErrorCarga, ErrorPlanificador, Opciones, Planificador,
    evaluar_carga,
};

fn modelos() -> PathBuf {
    std::env::var_os("LOCALL_MODELOS")
        .map_or_else(|| PathBuf::from(r"C:\Locall-poc\modelos"), PathBuf::from)
}

fn carga_de(modelo: PathBuf, contexto: u32) -> Option<Carga> {
    let motor = std::env::var_os("LOCALL_MOTOR").map_or_else(
        || PathBuf::from(r"C:\Locall-poc\motor\cpu\llama-server.exe"),
        PathBuf::from,
    );
    if !motor.is_file() {
        eprintln!("(prueba saltada: falta {})", motor.display());
        return None;
    }
    Some(Carga {
        modelo: "prueba".into(),
        config: ConfigMotor {
            ejecutable: motor,
            modelo,
            contexto,
            hilos: locall_system::hilos_para_motor(),
            capas_gpu: 0,
            paralelo: 1,
            registro: std::env::temp_dir().join(format!("locall-comp-{}.log", std::process::id())),
        },
    })
}

fn real(archivo: &str, contexto: u32) -> Option<Carga> {
    let ruta = modelos().join(archivo);
    if !ruta.is_file() {
        eprintln!("(prueba saltada: falta {})", ruta.display());
        return None;
    }
    carga_de(ruta, contexto)
}

/// Un GGUF v3 mínimo, sin tensores, con los metadatos de texto y `u16` indicados.
fn gguf_minimo(textos: &[(&str, &str)], numeros_u16: &[(&str, u16)]) -> Vec<u8> {
    let mut v = b"GGUF".to_vec();
    v.extend(3u32.to_le_bytes());
    v.extend(0u64.to_le_bytes());
    v.extend(((textos.len() + numeros_u16.len()) as u64).to_le_bytes());
    let cadena = |v: &mut Vec<u8>, s: &str| {
        v.extend((s.len() as u64).to_le_bytes());
        v.extend(s.as_bytes());
    };
    for (clave, valor) in textos {
        cadena(&mut v, clave);
        v.extend(8u32.to_le_bytes()); // cadena
        cadena(&mut v, valor);
    }
    for (clave, valor) in numeros_u16 {
        cadena(&mut v, clave);
        v.extend(2u32.to_le_bytes()); // u16
        v.extend(valor.to_le_bytes());
    }
    v
}

fn archivo_temporal(nombre: &str, contenido: &[u8]) -> PathBuf {
    let carpeta = std::env::temp_dir().join(format!("locall-comp-{}", std::process::id()));
    std::fs::create_dir_all(&carpeta).unwrap();
    let ruta = carpeta.join(nombre);
    std::fs::write(&ruta, contenido).unwrap();
    ruta
}

#[test]
fn evalua_los_modelos_reales() {
    let (Some(pequeno), Some(dividido)) = (
        real("qwen2.5-1.5b-instruct-q4_k_m.gguf", 4096),
        real("qwen2.5-7b-instruct-q4_k_m-00001-of-00002.gguf", 4096),
    ) else {
        return;
    };
    let e = evaluar_carga(&pequeno).unwrap();
    eprintln!(
        "1,5B: {:.2} GB, {:?}",
        e.estimacion.total as f64 / 1e9,
        e.encaje
    );
    assert!(e.comprobacion.es_compatible());

    // El 7B dividido: deben contarse las DOS partes.
    let e = evaluar_carga(&dividido).unwrap();
    let partes: u64 = ["00001", "00002"]
        .iter()
        .map(|n| {
            std::fs::metadata(
                modelos().join(format!("qwen2.5-7b-instruct-q4_k_m-{n}-of-00002.gguf")),
            )
            .unwrap()
            .len()
        })
        .sum();
    eprintln!(
        "7B: {:.2} GB, {:?}",
        e.estimacion.total as f64 / 1e9,
        e.encaje
    );
    assert_eq!(e.estimacion.pesos, partes);
}

#[test]
fn rechaza_al_instante_un_modelo_incompatible() {
    let ruta = archivo_temporal(
        "inventado.gguf",
        &gguf_minimo(&[("general.architecture", "inventada")], &[]),
    );
    let Some(carga) = carga_de(ruta, 4096) else {
        return;
    };
    let inicio = Instant::now();
    let error = ArrancadorReal::default().arrancar(&carga).unwrap_err();
    eprintln!("mensaje: {error}");
    assert!(matches!(error, ErrorCarga::Incompatible(_)), "{error:?}");
    assert!(
        inicio.elapsed() < Duration::from_millis(500),
        "no debería arrancar el motor"
    );
}

#[test]
fn rechaza_un_archivo_que_no_es_un_modelo() {
    let ruta = archivo_temporal("falso.gguf", b"esto no es un GGUF");
    let Some(carga) = carga_de(ruta, 4096) else {
        return;
    };
    let error = ArrancadorReal::default().arrancar(&carga).unwrap_err();
    eprintln!("mensaje: {error}");
    assert!(matches!(error, ErrorCarga::Cabecera(_)), "{error:?}");
}

#[test]
fn avisa_si_falta_una_parte() {
    let ruta = archivo_temporal(
        "partido-00001-of-00002.gguf",
        &gguf_minimo(&[("general.architecture", "qwen2")], &[("split.count", 2)]),
    );
    let Some(carga) = carga_de(ruta, 4096) else {
        return;
    };
    let error = ArrancadorReal::default().arrancar(&carga).unwrap_err();
    eprintln!("mensaje: {error}");
    match error {
        ErrorCarga::FaltaParte(parte) => {
            assert!(parte.ends_with("partido-00002-of-00002.gguf"));
        }
        otro => panic!("{otro:?}"),
    }
}

#[test]
fn el_planificador_devuelve_el_error_de_la_comprobacion() {
    let ruta = archivo_temporal(
        "inventado-plan.gguf",
        &gguf_minimo(&[("general.architecture", "inventada")], &[]),
    );
    let Some(carga) = carga_de(ruta, 4096) else {
        return;
    };
    let plan = Planificador::nuevo(ArrancadorReal::default(), Opciones { inactividad: None });
    let error = plan.usar(&carga).map(drop).unwrap_err();
    assert!(
        matches!(error, ErrorPlanificador::Motor(ErrorCarga::Incompatible(_))),
        "{error:?}"
    );
}

#[test]
fn carga_de_verdad_cuando_cabe() {
    let Some(carga) = real("qwen2.5-1.5b-instruct-q4_k_m.gguf", 2048) else {
        return;
    };
    let e = evaluar_carga(&carga).unwrap();
    if e.encaje == Encaje::NoCabe {
        eprintln!("(prueba saltada: ahora mismo no hay memoria libre para el 1,5B)");
        return;
    }
    let motor = ArrancadorReal::default().arrancar(&carga).unwrap();
    assert!(motor.pid() > 0);
    drop(motor);
    let _ = std::fs::remove_file(&carga.config.registro);
}
