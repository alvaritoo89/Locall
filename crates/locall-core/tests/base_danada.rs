//! A damaged database when opening the core (ADR 0029): it is set aside, a new one is used and the user is told.

mod comun;

use comun::{CarpetaDePrueba, nucleo};
use locall_config::Rutas;

#[test]
fn una_base_danada_se_aparta_y_se_avisa() {
    let carpeta = CarpetaDePrueba::nueva("base-danada");
    let ruta = Rutas::en(carpeta.0.clone()).base_de_datos();
    std::fs::create_dir_all(ruta.parent().unwrap()).unwrap();
    std::fs::write(&ruta, b"esto no es una base de datos".repeat(50)).unwrap();

    let n = nucleo(&carpeta, None);
    assert!(n.modelos().unwrap().is_empty(), "se empieza una base nueva");
    let aviso = n
        .avisos()
        .iter()
        .find(|a| a.contains("dañada"))
        .expect("debería avisar");
    assert!(aviso.contains(".danada-"), "dice dónde está: {aviso}");

    // The damaged file is still there, renamed.
    let apartados = std::fs::read_dir(ruta.parent().unwrap())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".danada-"))
        .count();
    assert_eq!(apartados, 1);
}

#[test]
fn una_base_sana_no_avisa() {
    let carpeta = CarpetaDePrueba::nueva("base-sana");
    drop(nucleo(&carpeta, None));
    let n = nucleo(&carpeta, None);
    assert!(!n.avisos().iter().any(|a| a.contains("dañada")));
}
