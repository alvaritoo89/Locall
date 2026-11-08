use std::path::PathBuf;

use super::*;

fn seg(t: &str) -> Segmento {
    Segmento::nuevo(t).unwrap()
}

fn huella(c: char) -> Huella {
    Huella::desde_hex(&c.to_string().repeat(64)).unwrap()
}

fn archivo(nombre: &str, c: char, bytes: u64) -> ArchivoModelo {
    ArchivoModelo {
        nombre: NombreArchivo::nuevo(nombre).unwrap(),
        huella: huella(c),
        bytes,
    }
}

fn modelo_4b() -> ModeloNuevo {
    ModeloNuevo {
        id: seg("qwen3-4b"),
        nombre: "Qwen3 4B".into(),
        origen: "hf:Qwen/Qwen3-4B-GGUF".into(),
        revision: "bc640142c66e1fdd12af0bd68f40445458f3869b".into(),
        cuantizacion: Some("Q4_K_M".into()),
        licencia: Some("apache-2.0".into()),
        arquitectura: Some("qwen3".into()),
        contexto_max: Some(40_960),
        verificado: true,
        archivos: vec![archivo("Qwen3-4B-Q4_K_M.gguf", 'a', 2_497_280_256)],
    }
}

fn modelo_7b_dividido() -> ModeloNuevo {
    ModeloNuevo {
        id: seg("qwen2.5-7b"),
        nombre: "Qwen2.5 7B".into(),
        origen: "hf:Qwen/Qwen2.5-7B-Instruct-GGUF".into(),
        revision: "bb5d59e06d9551d752d08b292a50eb208b07ab1f".into(),
        cuantizacion: Some("Q4_K_M".into()),
        licencia: Some("apache-2.0".into()),
        arquitectura: Some("qwen2".into()),
        contexto_max: Some(131_072),
        verificado: true,
        // Desordenados a propósito.
        archivos: vec![
            archivo(
                "qwen2.5-7b-instruct-q4_k_m-00002-of-00002.gguf",
                'c',
                689_872_288,
            ),
            archivo(
                "qwen2.5-7b-instruct-q4_k_m-00001-of-00002.gguf",
                'b',
                3_993_201_344,
            ),
        ],
    }
}

#[test]
fn registra_y_lee_un_modelo() {
    let mut r = Registro::en_memoria().unwrap();
    r.registrar(&modelo_4b(), 1_000).unwrap();

    let m = r.obtener(&seg("qwen3-4b")).unwrap().unwrap();
    assert_eq!(m.nombre, "Qwen3 4B");
    assert_eq!(m.licencia.as_deref(), Some("apache-2.0"));
    assert_eq!(m.contexto_max, Some(40_960));
    assert!(m.verificado);
    assert_eq!(m.instalado, 1_000);
    assert_eq!(m.ultimo_uso, None);
    assert_eq!(m.bytes_totales(), 2_497_280_256);
    assert_eq!(r.obtener(&seg("no-existe")).unwrap(), None);
}

#[test]
fn ordena_las_partes_de_un_modelo_dividido() {
    let mut r = Registro::en_memoria().unwrap();
    r.registrar(&modelo_7b_dividido(), 1).unwrap();
    let m = r.obtener(&seg("qwen2.5-7b")).unwrap().unwrap();
    assert!(m.archivos[0].nombre.como_str().contains("-00001-of-"));
    assert_eq!(m.bytes_totales(), 3_993_201_344 + 689_872_288);
}

#[test]
fn lista_por_nombre() {
    let mut r = Registro::en_memoria().unwrap();
    r.registrar(&modelo_4b(), 1).unwrap();
    r.registrar(&modelo_7b_dividido(), 1).unwrap();
    let nombres: Vec<String> = r.listar().unwrap().into_iter().map(|m| m.nombre).collect();
    assert_eq!(nombres, ["Qwen2.5 7B", "Qwen3 4B"]);
}

#[test]
fn rechaza_duplicados_y_modelos_sin_archivos() {
    let mut r = Registro::en_memoria().unwrap();
    r.registrar(&modelo_4b(), 1).unwrap();
    assert!(matches!(
        r.registrar(&modelo_4b(), 2),
        Err(ErrorRegistro::YaExiste(_))
    ));
    let mut vacio = modelo_7b_dividido();
    vacio.archivos.clear();
    assert!(matches!(
        r.registrar(&vacio, 1),
        Err(ErrorRegistro::SinArchivos)
    ));
}

#[test]
fn una_transaccion_fallida_no_deja_nada() {
    let mut r = Registro::en_memoria().unwrap();
    let mut roto = modelo_4b();
    // Dos archivos con el mismo nombre: el segundo INSERT falla a mitad de la transacción.
    roto.archivos.push(archivo("Qwen3-4B-Q4_K_M.gguf", 'b', 1));
    assert!(r.registrar(&roto, 1).is_err());
    assert_eq!(
        r.obtener(&seg("qwen3-4b")).unwrap(),
        None,
        "no debe quedar el modelo a medias"
    );
    assert!(r.huellas_en_uso().unwrap().is_empty());
}

#[test]
fn marca_el_ultimo_uso() {
    let mut r = Registro::en_memoria().unwrap();
    r.registrar(&modelo_4b(), 1).unwrap();
    r.marcar_uso(&seg("qwen3-4b"), 500).unwrap();
    assert_eq!(
        r.obtener(&seg("qwen3-4b")).unwrap().unwrap().ultimo_uso,
        Some(500)
    );
    assert!(matches!(
        r.marcar_uso(&seg("otro"), 1),
        Err(ErrorRegistro::NoEncontrado(_))
    ));
}

#[test]
fn eliminar_devuelve_las_huellas_y_borra_sus_archivos() {
    let mut r = Registro::en_memoria().unwrap();
    r.registrar(&modelo_4b(), 1).unwrap();
    r.registrar(&modelo_7b_dividido(), 1).unwrap();
    assert_eq!(r.huellas_en_uso().unwrap().len(), 3);

    let huellas = r.eliminar(&seg("qwen2.5-7b")).unwrap();
    assert_eq!(huellas, vec![huella('b'), huella('c')]);
    assert_eq!(r.huellas_en_uso().unwrap(), BTreeSet::from([huella('a')]));
    assert!(matches!(
        r.eliminar(&seg("qwen2.5-7b")),
        Err(ErrorRegistro::NoEncontrado(_))
    ));
}

/// Archivo de base de datos temporal que se borra al terminar (junto con los de WAL).
struct BaseDeDatosDePrueba(PathBuf);

impl BaseDeDatosDePrueba {
    fn nueva(nombre: &str) -> BaseDeDatosDePrueba {
        let ruta = std::env::temp_dir().join(format!(
            "locall-registro-{nombre}-{}.db",
            std::process::id()
        ));
        let b = BaseDeDatosDePrueba(ruta);
        b.borrar();
        b
    }
    fn borrar(&self) {
        for sufijo in ["", "-wal", "-shm"] {
            let mut ruta = self.0.clone().into_os_string();
            ruta.push(sufijo);
            let _ = std::fs::remove_file(ruta);
        }
    }
}

impl Drop for BaseDeDatosDePrueba {
    fn drop(&mut self) {
        self.borrar();
    }
}

#[test]
fn los_datos_persisten_al_cerrar_y_abrir() {
    let db = BaseDeDatosDePrueba::nueva("persiste");
    {
        let mut r = Registro::abrir(&db.0).unwrap();
        assert_eq!(r.version_esquema().unwrap(), VERSION_ESQUEMA);
        r.registrar(&modelo_4b(), 1).unwrap();
    } // se cierra
    let r = Registro::abrir(&db.0).unwrap(); // abrir otra vez NO repite las migraciones
    assert_eq!(r.version_esquema().unwrap(), VERSION_ESQUEMA);
    assert_eq!(r.listar().unwrap().len(), 1);
}

#[test]
fn no_toca_una_base_de_datos_de_una_version_futura() {
    let db = BaseDeDatosDePrueba::nueva("futura");
    {
        let c = Connection::open(&db.0).unwrap();
        c.pragma_update(None, "user_version", 99).unwrap();
    }
    assert!(matches!(
        Registro::abrir(&db.0),
        Err(ErrorRegistro::VersionFutura { encontrada: 99, .. })
    ));
}

#[test]
fn detecta_datos_manipulados() {
    let mut r = Registro::en_memoria().unwrap();
    r.registrar(&modelo_4b(), 1).unwrap();
    // Alguien cambia la huella a mano por 64 caracteres que no son hexadecimales.
    r.conexion
        .execute("UPDATE archivos SET huella = ?1", [&"z".repeat(64)])
        .unwrap();
    assert!(matches!(
        r.obtener(&seg("qwen3-4b")),
        Err(ErrorRegistro::DatoInvalido { .. })
    ));
}
