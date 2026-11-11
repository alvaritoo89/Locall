//! Tests of Locall's settings.

use std::path::PathBuf;
use std::time::Duration;

use locall_config::lector::Valor;
use locall_config::{
    ARCHIVO_POR_DEFECTO, Aviso, Config, ErrorConfig, Hilos, Rutas, texto_con_cambios,
};

struct CarpetaDePrueba(PathBuf);

impl CarpetaDePrueba {
    fn nueva(nombre: &str) -> CarpetaDePrueba {
        let ruta =
            std::env::temp_dir().join(format!("locall-config-{nombre}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ruta);
        CarpetaDePrueba(ruta)
    }
}

impl Drop for CarpetaDePrueba {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The error message for a configuration text.
fn error_de(texto: &str) -> String {
    Config::desde_texto(texto).unwrap_err().to_string()
}

#[test]
fn el_archivo_por_defecto_da_los_valores_por_defecto() {
    let (config, avisos) = Config::desde_texto(ARCHIVO_POR_DEFECTO).unwrap();
    assert_eq!(config, Config::default());
    assert!(avisos.is_empty());
}

#[test]
fn lee_unos_ajustes_cambiados() {
    let texto = "carpeta_modelos = 'D:\\Modelos IA'\r\nhilos = 4\r\ncontexto = 8_192\r\n\
                 inactividad_minutos = 0\r\nparalelo = 2\r\nregistros_dias = 30\r\npuerto = 12000\r\n";
    let (config, _) = Config::desde_texto(texto).unwrap();
    assert_eq!(
        config.carpeta_modelos,
        Some(PathBuf::from(r"D:\Modelos IA"))
    );
    assert_eq!(config.hilos, Hilos::Numero(4));
    assert_eq!(config.hilos_para_motor(), 4);
    assert_eq!(config.contexto, 8192);
    assert_eq!(config.inactividad(), None, "0 = no descargar nunca");
    assert_eq!(config.paralelo, 2);
    assert_eq!(config.registros_dias, 30);
    assert_eq!(config.puerto, 12000);
}

#[test]
fn los_valores_no_validos_se_explican() {
    // The Phase 0 bug: far more threads than the machine had. Here, a number no computer has.
    let m = error_de("hilos = 100000");
    assert!(
        m.contains("línea 1") && m.contains("hilos") && m.contains("entre 1 y"),
        "{m}"
    );

    let m = error_de("\ncontexto = 10");
    assert!(
        m.contains("línea 2") && m.contains("entre 256 y 1048576"),
        "{m}"
    );

    let m = error_de("puerto = 80");
    assert!(m.contains("entre 1024 y 65535"), "{m}");

    let m = error_de("paralelo = 0");
    assert!(m.contains("entre 1 y 16"), "{m}");

    let m = error_de("hilos = 'muchos'");
    assert!(m.contains("\"auto\" o un número"), "{m}");

    let m = error_de("contexto = '4096'");
    assert!(m.contains("sin comillas"), "{m}");

    let m = error_de("carpeta_modelos = 'modelos'");
    assert!(m.contains("ruta completa"), "{m}");

    let m = error_de("carpeta_modelos = \"C:\\Modelos\"");
    assert!(m.contains("comillas simples"), "{m}");
}

#[test]
fn una_clave_desconocida_es_un_aviso() {
    let (config, avisos) = Config::desde_texto("contexto = 2048\ncontexo = 9999\n").unwrap();
    assert_eq!(config.contexto, 2048);
    assert_eq!(
        avisos,
        [Aviso::ClaveDesconocida {
            linea: 2,
            clave: "contexo".into()
        }]
    );
    assert!(avisos[0].to_string().contains("«contexo» no existe"));
}

#[test]
fn hilos_auto_usa_todos() {
    let (config, _) = Config::desde_texto("hilos = 'AUTO'").unwrap();
    assert_eq!(config.hilos, Hilos::Auto);
    assert_eq!(config.hilos_para_motor(), locall_system::hilos_para_motor());
}

#[test]
fn inactividad_en_minutos() {
    let config = Config::default();
    assert_eq!(config.inactividad(), Some(Duration::from_secs(5 * 60)));
}

#[test]
fn las_rutas_cuelgan_de_la_carpeta_de_datos() {
    let rutas = Rutas::en(r"C:\Datos\Locall");
    assert_eq!(
        rutas.config(),
        PathBuf::from(r"C:\Datos\Locall\config.toml")
    );
    assert_eq!(
        rutas.base_de_datos(),
        PathBuf::from(r"C:\Datos\Locall\locall.db")
    );
    assert_eq!(
        rutas.registros(),
        PathBuf::from(r"C:\Datos\Locall\registros")
    );
    assert_eq!(
        Config::default().carpeta_modelos(&rutas),
        PathBuf::from(r"C:\Datos\Locall\modelos")
    );
}

#[test]
fn la_carpeta_por_defecto_esta_en_appdata_local() {
    // Only if the test is not run with LOCALL_DATOS (which replaces it).
    if std::env::var_os("LOCALL_DATOS").is_some() {
        return;
    }
    let rutas = Rutas::por_defecto().unwrap();
    let local = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap());
    assert_eq!(rutas.datos(), local.join("Locall"));
    assert!(!rutas.datos().to_string_lossy().contains("Roaming"));
}

#[test]
fn sin_archivo_se_usan_los_valores_por_defecto() {
    let carpeta = CarpetaDePrueba::nueva("sin-archivo");
    let (config, avisos) = Config::leer(&carpeta.0.join("config.toml")).unwrap();
    assert_eq!(config, Config::default());
    assert!(avisos.is_empty());
}

#[test]
fn crea_el_archivo_pero_nunca_pisa_el_del_usuario() {
    let carpeta = CarpetaDePrueba::nueva("crear");
    let rutas = Rutas::en(carpeta.0.clone());

    assert!(
        Config::crear_si_no_existe(&rutas).unwrap(),
        "la primera vez lo crea"
    );
    assert_eq!(
        std::fs::read_to_string(rutas.config()).unwrap(),
        ARCHIVO_POR_DEFECTO
    );

    // The user changes it…
    std::fs::write(rutas.config(), "contexto = 2048\n").unwrap();
    // …and Locall neither recreates nor overwrites it.
    assert!(!Config::crear_si_no_existe(&rutas).unwrap());
    let (config, _) = Config::leer(&rutas.config()).unwrap();
    assert_eq!(config.contexto, 2048);
}

#[test]
fn un_archivo_que_no_es_texto_se_explica() {
    let carpeta = CarpetaDePrueba::nueva("binario");
    std::fs::create_dir_all(&carpeta.0).unwrap();
    let ruta = carpeta.0.join("config.toml");
    std::fs::write(&ruta, [0xFF, 0xFE, 0x00, 0x41]).unwrap(); // UTF-16, as some editors save it
    let error = Config::leer(&ruta).unwrap_err();
    assert!(matches!(error, ErrorConfig::Sintaxis { .. }), "{error:?}");
    assert!(error.to_string().contains("UTF-8"));
}

// ─── Saving changes ───────────────────────────────────────────────────────────

#[test]
fn guardar_cambia_solo_esas_lineas_y_respeta_los_comentarios() {
    let texto = "# mis notas\r\ncontexto = 2048 # pequeño\r\n\r\n# otra nota\r\npuerto = 11999\r\n";
    let nuevo = texto_con_cambios(
        texto,
        &[
            ("contexto", Valor::Entero(8192)),
            ("hilos", Valor::Texto("auto".into())),
        ],
    )
    .unwrap();
    assert_eq!(
        nuevo,
        "# mis notas\r\ncontexto = 8192\r\n\r\n# otra nota\r\npuerto = 11999\r\nhilos = \"auto\"\r\n",
        "mismos saltos de línea (\\r\\n), comentarios intactos y lo que faltaba, al final"
    );
    let (config, _) = Config::desde_texto(&nuevo).unwrap();
    assert_eq!(config.contexto, 8192);
    assert_eq!(config.puerto, 11999);
}

#[test]
fn guardar_solo_admite_los_ajustes_editables() {
    for clave in ["puerto", "carpeta_modelos", "paralelo", "inventado"] {
        let error = texto_con_cambios("", &[(clave, Valor::Entero(1))]).unwrap_err();
        assert!(
            matches!(error, ErrorConfig::NoEditable(_)),
            "{clave}: {error}"
        );
    }
}

#[test]
fn guardar_valida_antes_de_escribir() {
    let carpeta = CarpetaDePrueba::nueva("guardar-validar");
    let rutas = Rutas::en(&carpeta.0);
    Config::crear_si_no_existe(&rutas).unwrap();
    let antes = std::fs::read_to_string(rutas.config()).unwrap();

    let error = Config::guardar_cambios(&rutas, &[("contexto", Valor::Entero(10))]).unwrap_err();
    assert!(error.to_string().contains("contexto"), "{error}");
    assert_eq!(
        std::fs::read_to_string(rutas.config()).unwrap(),
        antes,
        "el archivo no se tocó"
    );

    let guardada =
        Config::guardar_cambios(&rutas, &[("inactividad_minutos", Valor::Entero(12))]).unwrap();
    assert_eq!(guardada.inactividad_minutos, 12);
    let (leida, _) = Config::leer(&rutas.config()).unwrap();
    assert_eq!(leida, guardada);
    assert!(
        !rutas.config().with_extension("toml.nuevo").exists(),
        "no queda el temporal"
    );
    let despues = std::fs::read_to_string(rutas.config()).unwrap();
    assert!(
        despues.contains("# Contexto por defecto"),
        "los comentarios siguen ahí"
    );
}

#[test]
fn guardar_sin_archivo_lo_crea_desde_el_de_por_defecto() {
    let carpeta = CarpetaDePrueba::nueva("guardar-nuevo");
    let rutas = Rutas::en(&carpeta.0);
    let guardada =
        Config::guardar_cambios(&rutas, &[("registros_dias", Valor::Entero(30))]).unwrap();
    assert_eq!(guardada.registros_dias, 30);
    let texto = std::fs::read_to_string(rutas.config()).unwrap();
    assert!(texto.starts_with("# Ajustes de Locall."));
    assert!(texto.contains("registros_dias = 30"));
}
