//! The real log writer (it is global: a single `iniciar` per process, which is why it is a single test).

use locall_config::registros::{self, ErrorRegistros};

#[test]
fn guarda_solo_lo_nuestro_y_en_el_archivo_del_dia() {
    let carpeta = std::env::temp_dir().join(format!("locall-reg-iniciar-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&carpeta);

    registros::iniciar(&carpeta, 7).unwrap();
    log::info!(target: "locall_prueba", "modelo cargado: qwen (1.0 s)");
    log::warn!(target: "locall_prueba", "reintento de descarga");
    log::debug!(target: "locall_prueba", "detalle que no se guarda");
    log::info!(target: "ureq", "mensaje de otro crate que no se guarda");
    log::logger().flush();

    let hoy = locall_system::fecha_hora_local().unwrap().fecha();
    let texto = std::fs::read_to_string(carpeta.join(registros::nombre_del_dia(&hoy))).unwrap();
    let lineas: Vec<&str> = texto.lines().collect();
    assert_eq!(lineas.len(), 2, "{texto}");
    assert!(lineas[0].starts_with(&hoy));
    assert!(
        lineas[0].ends_with("INFO  locall_prueba: modelo cargado: qwen (1.0 s)"),
        "{}",
        lineas[0]
    );
    assert!(lineas[1].contains("WARN  locall_prueba: reintento de descarga"));

    // There can only be one writer per process.
    assert!(matches!(
        registros::iniciar(&carpeta, 7),
        Err(ErrorRegistros::YaIniciados)
    ));
    let _ = std::fs::remove_dir_all(&carpeta);
}
