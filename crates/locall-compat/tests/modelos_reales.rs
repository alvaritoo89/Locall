//! Los tres modelos reales de la Fase 0 deben ser compatibles con el motor fijado (build 12189), que los cargó.
//! Se salta si no están (ver `LOCALL_MODELOS`).

use std::path::PathBuf;

#[test]
fn los_modelos_de_la_fase_0_son_compatibles() {
    let carpeta = std::env::var_os("LOCALL_MODELOS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Locall-poc\modelos"));
    for archivo in [
        "qwen2.5-1.5b-instruct-q4_k_m.gguf",
        "Qwen3-4B-Q4_K_M.gguf",
        "qwen2.5-7b-instruct-q4_k_m-00001-of-00002.gguf",
    ] {
        let ruta = carpeta.join(archivo);
        if !ruta.exists() {
            eprintln!("(saltado: no encuentro {})", ruta.display());
            continue;
        }
        let cabecera = locall_gguf::leer_archivo(&ruta).unwrap();
        let r = locall_compat::comprobar(&cabecera, 4_096);
        assert!(r.es_compatible(), "{archivo}: {:?}", r.problemas);
        assert!(r.avisos.is_empty(), "{archivo}: {:?}", r.avisos);
    }
}
