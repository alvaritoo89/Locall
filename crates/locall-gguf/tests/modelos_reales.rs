//! Tests with the real Phase 0 models.
//!
//! The models weigh gigabytes and are not in the repository. They are looked up in the folder given by the
//! `LOCALL_MODELOS` environment variable or, if it is not set, in `C:\Locall-poc\modelos`. If they are not there, the
//! test is skipped with a notice. The expected values are the ones measured by the Phase 0 Python script (and they
//! match Hugging Face).

use std::path::PathBuf;

struct Esperado {
    archivo: &'static str,
    bytes_cabecera: u64,
    arquitectura: &'static str,
    capas: u64,
    contexto_max: u64,
    cabezas: u64,
    cabezas_kv: u64,
    vocabulario: u64,
    partes: Option<u64>,
    cache_kv_4k_mib: u64,
}

const MODELOS: [Esperado; 3] = [
    Esperado {
        archivo: "qwen2.5-1.5b-instruct-q4_k_m.gguf",
        bytes_cabecera: 5_950_493,
        arquitectura: "qwen2",
        capas: 28,
        contexto_max: 32_768,
        cabezas: 12,
        cabezas_kv: 2,
        vocabulario: 151_936,
        partes: None,
        cache_kv_4k_mib: 112,
    },
    Esperado {
        archivo: "Qwen3-4B-Q4_K_M.gguf",
        bytes_cabecera: 5_956_328,
        arquitectura: "qwen3",
        capas: 36,
        contexto_max: 40_960,
        cabezas: 32,
        cabezas_kv: 8,
        vocabulario: 151_936,
        partes: None,
        cache_kv_4k_mib: 576,
    },
    Esperado {
        archivo: "qwen2.5-7b-instruct-q4_k_m-00001-of-00002.gguf",
        bytes_cabecera: 5_950_142,
        arquitectura: "qwen2",
        capas: 28,
        contexto_max: 131_072,
        cabezas: 28,
        cabezas_kv: 4,
        vocabulario: 152_064,
        partes: Some(2),
        cache_kv_4k_mib: 224,
    },
];

fn carpeta_modelos() -> PathBuf {
    std::env::var_os("LOCALL_MODELOS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Locall-poc\modelos"))
}

#[test]
fn lee_los_modelos_de_la_fase_0() {
    let carpeta = carpeta_modelos();
    let mut probados = 0;

    for e in &MODELOS {
        let ruta = carpeta.join(e.archivo);
        if !ruta.exists() {
            eprintln!("(saltado: no encuentro {})", ruta.display());
            continue;
        }
        let cabecera = locall_gguf::leer_archivo(&ruta).expect("la cabecera debería leerse");
        let r = cabecera.resumen();

        assert_eq!(cabecera.bytes_cabecera, e.bytes_cabecera, "{}", e.archivo);
        assert_eq!(
            r.arquitectura.as_deref(),
            Some(e.arquitectura),
            "{}",
            e.archivo
        );
        assert_eq!(r.capas, Some(e.capas), "{}", e.archivo);
        assert_eq!(r.contexto_max, Some(e.contexto_max), "{}", e.archivo);
        assert_eq!(r.cabezas, Some(e.cabezas), "{}", e.archivo);
        assert_eq!(r.cabezas_kv, Some(e.cabezas_kv), "{}", e.archivo);
        assert_eq!(r.dim_clave, Some(128), "{}", e.archivo);
        assert_eq!(r.vocabulario, Some(e.vocabulario), "{}", e.archivo);
        assert_eq!(r.partes, e.partes, "{}", e.archivo);
        assert!(
            r.plantilla_chat,
            "{} debería traer plantilla de chat",
            e.archivo
        );
        assert_eq!(
            cabecera.cache_kv_bytes(4_096, 2),
            Some(e.cache_kv_4k_mib * 1024 * 1024),
            "{}",
            e.archivo
        );
        probados += 1;
    }

    eprintln!(
        "Modelos reales comprobados: {probados} de {}",
        MODELOS.len()
    );
}
