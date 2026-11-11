//! Installing, listing and removing models against a fake Hugging Face (no internet).

mod comun;

use std::sync::atomic::AtomicBool;

use comun::{Archivo, CarpetaDePrueba, Repo, gguf, hf_falso, nucleo};
use locall_core::{ErrorNucleo, PeticionInstalar};

fn archivo(ruta: &str, datos: Vec<u8>) -> Archivo {
    Archivo {
        ruta: ruta.into(),
        datos,
        con_huella: true,
    }
}

fn repo(nombre: &str, archivos: Vec<Archivo>) -> Repo {
    Repo {
        nombre: nombre.into(),
        archivos,
        licencia: Some("mit".into()),
        protegido: false,
    }
}

/// A small "llama" model (300 KB) the engine would recognize.
fn modelo_llama(relleno: usize) -> Vec<u8> {
    gguf(
        &[
            ("general.architecture", "llama"),
            ("tokenizer.chat_template", "{{ x }}"),
        ],
        &[],
        relleno,
    )
}

fn pedir(repo: &str, archivo: &str) -> PeticionInstalar {
    PeticionInstalar {
        repo: repo.into(),
        revision: "main".into(),
        archivo: archivo.into(),
        id: None,
    }
}

fn instalar(
    nucleo: &locall_core::Nucleo,
    peticion: &PeticionInstalar,
) -> Result<locall_registry::Modelo, ErrorNucleo> {
    nucleo.instalar(peticion, &AtomicBool::new(false), |_| {})
}

#[test]
fn instala_un_modelo_fijado_al_commit() {
    let datos = modelo_llama(300_000);
    let hf = hf_falso(vec![repo(
        "prueba/pequeno",
        vec![
            archivo("README.md", b"hola".to_vec()),
            archivo("Pequeno-Q4_K_M.gguf", datos.clone()),
        ],
    )]);
    let carpeta = CarpetaDePrueba::nueva("instalar");
    let nucleo = nucleo(&carpeta, Some(&hf));
    let mut ultimo = None;

    let modelo = nucleo
        .instalar(
            &pedir("prueba/pequeno", "Pequeno-Q4_K_M.gguf"),
            &AtomicBool::new(false),
            |p| ultimo = Some(p),
        )
        .unwrap();

    assert_eq!(modelo.id.to_string(), "pequeno-q4_k_m");
    assert_eq!(modelo.nombre, "Pequeno-Q4_K_M");
    assert_eq!(modelo.origen, "hf:prueba/pequeno");
    assert_eq!(
        modelo.revision,
        comun::COMMIT,
        "fijado al commit, no a «main»"
    );
    assert_eq!(modelo.cuantizacion.as_deref(), Some("Q4_K_M"));
    assert_eq!(modelo.licencia.as_deref(), Some("mit"));
    assert_eq!(modelo.arquitectura.as_deref(), Some("llama"));
    assert_eq!(modelo.bytes_totales(), datos.len() as u64);
    let ultimo = ultimo.unwrap();
    assert_eq!((ultimo.parte, ultimo.partes), (1, 1));
    assert_eq!(ultimo.descargados, ultimo.total);

    // It appears in the list and the engine would find it in its folder.
    assert_eq!(nucleo.modelos().unwrap().len(), 1);
    let carga = nucleo.carga("pequeno-q4_k_m").unwrap();
    assert_eq!(std::fs::read(&carga.config.modelo).unwrap(), datos);

    // What was requested: revision, list, header and the file (all pinned to the commit).
    let pedidas = hf.pedidas.lock().unwrap().clone();
    assert!(pedidas.iter().any(|r| r.ends_with("/revision/main")));
    assert!(
        pedidas.iter().all(|r| !r.contains("/resolve/main/")),
        "{pedidas:?}"
    );
}

#[test]
fn instala_todas_las_partes_de_un_modelo_dividido() {
    let parte = |n: u8| {
        let mut d = gguf(
            &[("general.architecture", "llama")],
            &[("split.count", 2)],
            50_000,
        );
        d.push(n); // different content in each part
        d
    };
    let hf = hf_falso(vec![repo(
        "prueba/dividido",
        vec![
            archivo("Q4_K_M/grande-q4_k_m-00001-of-00002.gguf", parte(1)),
            archivo("Q4_K_M/grande-q4_k_m-00002-of-00002.gguf", parte(2)),
        ],
    )]);
    let carpeta = CarpetaDePrueba::nueva("dividido");
    let nucleo = nucleo(&carpeta, Some(&hf));

    // It can be requested by any of its parts, with or without a folder.
    let modelo = instalar(
        &nucleo,
        &pedir("prueba/dividido", "grande-q4_k_m-00002-of-00002.gguf"),
    )
    .unwrap();

    assert_eq!(modelo.id.to_string(), "grande-q4_k_m");
    let nombres: Vec<String> = modelo
        .archivos
        .iter()
        .map(|a| a.nombre.to_string())
        .collect();
    assert_eq!(
        nombres,
        [
            "grande-q4_k_m-00001-of-00002.gguf",
            "grande-q4_k_m-00002-of-00002.gguf"
        ]
    );
    // The engine receives the first part, and the second one is next to it with its original name.
    let carga = nucleo.carga("grande-q4_k_m").unwrap();
    assert!(
        carga
            .config
            .modelo
            .ends_with("grande-q4_k_m-00001-of-00002.gguf")
    );
    assert!(
        carga
            .config
            .modelo
            .with_file_name("grande-q4_k_m-00002-of-00002.gguf")
            .is_file()
    );
}

#[test]
fn no_descarga_un_modelo_incompatible() {
    let hf = hf_falso(vec![repo(
        "prueba/raro",
        vec![archivo(
            "raro.gguf",
            gguf(&[("general.architecture", "inventada")], &[], 500_000),
        )],
    )]);
    let carpeta = CarpetaDePrueba::nueva("incompatible");
    let nucleo = nucleo(&carpeta, Some(&hf));

    let error = instalar(&nucleo, &pedir("prueba/raro", "raro.gguf")).unwrap_err();
    assert!(matches!(error, ErrorNucleo::Incompatible(_)), "{error:?}");
    assert!(error.to_string().contains("«inventada»"), "{error}");
    // Only the header was read: there is nothing in the store, not even half-done.
    let blobs = carpeta.0.join("modelos").join("blobs");
    assert_eq!(std::fs::read_dir(blobs).unwrap().count(), 0);
    let tmp = carpeta.0.join("modelos").join("tmp");
    assert_eq!(std::fs::read_dir(tmp).unwrap().count(), 0);
    assert!(nucleo.modelos().unwrap().is_empty());
}

#[test]
fn explica_lo_que_no_se_puede_instalar() {
    let mut sin_huella = archivo("sin-huella.gguf", modelo_llama(10));
    sin_huella.con_huella = false;
    let mut protegido = repo(
        "prueba/protegido",
        vec![archivo("p.gguf", modelo_llama(10))],
    );
    protegido.protegido = true;
    let hf = hf_falso(vec![
        repo(
            "prueba/varios",
            vec![
                archivo("a.gguf", modelo_llama(10)),
                sin_huella,
                archivo("roto-00001-of-00002.gguf", modelo_llama(10)),
            ],
        ),
        protegido,
    ]);
    let carpeta = CarpetaDePrueba::nueva("errores");
    let nucleo = nucleo(&carpeta, Some(&hf));

    let casos: Vec<(PeticionInstalar, &str)> = vec![
        (pedir("prueba/varios", "no-esta.gguf"), "tiene: a.gguf"),
        (
            pedir("prueba/varios", "sin-huella.gguf"),
            "no tiene huella sha256",
        ),
        (
            pedir("prueba/varios", "roto-00001-of-00002.gguf"),
            "falta la parte",
        ),
        (
            pedir("prueba/no-existe", "x.gguf"),
            "no tiene el repositorio",
        ),
        (pedir("prueba/protegido", "p.gguf"), "aceptar condiciones"),
        (pedir("sin-barra", "x.gguf"), "propietario/nombre"),
    ];
    for (peticion, fragmento) in casos {
        let error = instalar(&nucleo, &peticion).unwrap_err();
        assert!(
            error.to_string().contains(fragmento),
            "{}: {error}",
            peticion.archivo
        );
    }

    let mut id_malo = pedir("prueba/varios", "a.gguf");
    id_malo.id = Some("Con Espacios".into());
    assert!(matches!(
        instalar(&nucleo, &id_malo),
        Err(ErrorNucleo::IdInvalido(_))
    ));
}

#[test]
fn no_instala_dos_veces_el_mismo_identificador() {
    let hf = hf_falso(vec![repo(
        "prueba/m",
        vec![archivo("m.gguf", modelo_llama(1000))],
    )]);
    let carpeta = CarpetaDePrueba::nueva("dos-veces");
    let nucleo = nucleo(&carpeta, Some(&hf));
    instalar(&nucleo, &pedir("prueba/m", "m.gguf")).unwrap();
    let error = instalar(&nucleo, &pedir("prueba/m", "m.gguf")).unwrap_err();
    assert!(matches!(error, ErrorNucleo::YaInstalado(_)), "{error:?}");
}

#[test]
fn eliminar_libera_solo_lo_que_nadie_usa() {
    let compartido = modelo_llama(200_000);
    let hf = hf_falso(vec![repo(
        "prueba/m",
        vec![archivo("m.gguf", compartido.clone())],
    )]);
    let carpeta = CarpetaDePrueba::nueva("eliminar");
    let nucleo = nucleo(&carpeta, Some(&hf));

    // The same file installed under two names: a single blob in the store.
    instalar(&nucleo, &pedir("prueba/m", "m.gguf")).unwrap();
    let mut otro = pedir("prueba/m", "m.gguf");
    otro.id = Some("copia".into());
    instalar(&nucleo, &otro).unwrap();

    // Removing one does not delete the file the other uses.
    assert_eq!(nucleo.eliminar("m").unwrap(), 0);
    assert!(nucleo.carga("copia").unwrap().config.modelo.is_file());
    // Removing the last one does free it.
    assert_eq!(nucleo.eliminar("copia").unwrap(), compartido.len() as u64);
    assert!(nucleo.modelos().unwrap().is_empty());
    assert!(matches!(
        nucleo.eliminar("copia"),
        Err(ErrorNucleo::NoInstalado(_))
    ));
}

#[test]
fn una_instalacion_cancelada_continua_despues() {
    let datos = modelo_llama(2_000_000);
    let hf = hf_falso(vec![repo(
        "prueba/m",
        vec![archivo("m.gguf", datos.clone())],
    )]);
    let carpeta = CarpetaDePrueba::nueva("cancelar");
    let nucleo = nucleo(&carpeta, Some(&hf));

    let cancelar = AtomicBool::new(false);
    let error = nucleo
        .instalar(&pedir("prueba/m", "m.gguf"), &cancelar, |p| {
            if p.descargados > 300_000 {
                cancelar.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        })
        .unwrap_err();
    assert!(matches!(error, ErrorNucleo::Cancelado), "{error:?}");
    assert!(nucleo.modelos().unwrap().is_empty());

    // When requested again, it continues (with Range) and finishes.
    let mut primero = None;
    nucleo
        .instalar(&pedir("prueba/m", "m.gguf"), &AtomicBool::new(false), |p| {
            primero.get_or_insert(p.descargados);
        })
        .unwrap();
    assert!(primero.unwrap() > 300_000, "no empezó de cero: {primero:?}");
}
