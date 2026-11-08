//! Prueba completa con el modelo real dividido en dos partes (Qwen2.5 7B, 4,4 GB):
//! adoptar las dos partes en un almacén (sin copiarlas: enlaces duros), verificar sus huellas contra las de
//! Hugging Face, crear la snapshot y comprobar con locall-gguf que el archivo principal es el modelo correcto.
//! Se salta si los modelos no están (ver `LOCALL_MODELOS`).

use std::fs;
use std::path::PathBuf;

use locall_store::{Almacen, Huella, NombreArchivo, Segmento};

const PARTE_1: &str = "qwen2.5-7b-instruct-q4_k_m-00001-of-00002.gguf";
const PARTE_2: &str = "qwen2.5-7b-instruct-q4_k_m-00002-of-00002.gguf";
// Huellas publicadas en Hugging Face (Qwen/Qwen2.5-7B-Instruct-GGUF, revisión bb5d59e0).
const HUELLA_1: &str = "dfce12e3862a5283ccfb88221b48480e58745165de856439950d0f22590580db";
const HUELLA_2: &str = "539cf93f78e887edea1c04e2d7d8cdaca9d01dae9c9025bcb8accbe29df3d72a";

#[test]
fn snapshot_del_modelo_7b_dividido() {
    let modelos = std::env::var_os("LOCALL_MODELOS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Locall-poc\modelos"));
    let (origen_1, origen_2) = (modelos.join(PARTE_1), modelos.join(PARTE_2));
    if !origen_1.exists() || !origen_2.exists() {
        eprintln!(
            "(saltado: no encuentro las dos partes del 7B en {})",
            modelos.display()
        );
        return;
    }

    // Almacén temporal en la carpeta temporal del sistema (mismo disco que los modelos, para poder enlazar).
    let raiz = std::env::temp_dir().join(format!("locall-store-7b-{}", std::process::id()));
    let _ = fs::remove_dir_all(&raiz);
    let almacen = Almacen::abrir(&raiz).unwrap();

    let esperada_1 = Huella::desde_hex(HUELLA_1).unwrap();
    let esperada_2 = Huella::desde_hex(HUELLA_2).unwrap();
    let h1 = almacen
        .adoptar_archivo(&origen_1, Some(&esperada_1))
        .unwrap();
    let h2 = almacen
        .adoptar_archivo(&origen_2, Some(&esperada_2))
        .unwrap();

    let snap = almacen
        .crear_snapshot(
            &Segmento::nuevo("qwen2.5-7b-instruct").unwrap(),
            &Segmento::nuevo("bb5d59e0").unwrap(),
            &[
                (NombreArchivo::nuevo(PARTE_2).unwrap(), h2),
                (NombreArchivo::nuevo(PARTE_1).unwrap(), h1),
            ],
        )
        .unwrap();

    assert!(snap.ruta_principal().ends_with(PARTE_1));
    let cabecera = locall_gguf::leer_archivo(snap.ruta_principal()).unwrap();
    let resumen = cabecera.resumen();
    assert_eq!(resumen.arquitectura.as_deref(), Some("qwen2"));
    assert_eq!(resumen.partes, Some(2));

    // Limpieza: borrar el almacén temporal NO borra los modelos originales (son enlaces duros).
    fs::remove_dir_all(&raiz).unwrap();
    assert!(
        origen_1.exists() && origen_2.exists(),
        "los modelos originales deben seguir ahí"
    );
}
