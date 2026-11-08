//! Comprueba la huella de un modelo real contra la publicada por Hugging Face.
//! Se salta si el modelo no está (ver `LOCALL_MODELOS` en las pruebas de locall-gguf).

use std::path::PathBuf;

#[test]
fn la_huella_del_modelo_1_5b_coincide_con_hugging_face() {
    let carpeta = std::env::var_os("LOCALL_MODELOS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Locall-poc\modelos"));
    let ruta = carpeta.join("qwen2.5-1.5b-instruct-q4_k_m.gguf");
    if !ruta.exists() {
        eprintln!("(saltado: no encuentro {})", ruta.display());
        return;
    }
    // Huella publicada en Hugging Face (Qwen/Qwen2.5-1.5B-Instruct-GGUF, revisión 91cad511).
    let esperada = locall_store::Huella::desde_hex(
        "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e",
    )
    .unwrap();

    let calculada = locall_store::huella_de_archivo(&ruta).unwrap();

    assert_eq!(calculada, esperada);
}
