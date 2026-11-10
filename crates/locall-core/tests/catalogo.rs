//! El catálogo en el núcleo, contra un Hugging Face (y un GitHub) falsos: cargar, actualizar sin aceptar catálogos
//! antiguos ni mal firmados, e instalar por nombre comprobando antes lo firmado.

mod comun;

use std::sync::atomic::AtomicBool;

use comun::{
    Archivo, CarpetaDePrueba, Repo, gguf, hf_falso, nucleo, nucleo_con_catalogo, servir_catalogo,
};
use locall_core::{Actualizacion, ErrorCatalogo, ErrorNucleo, OrigenCatalogo};

const V1: &[u8] = include_bytes!("datos/v1.json");
const V1_FIRMA: &[u8] = include_bytes!("datos/v1.json.minisig");
const V2: &[u8] = include_bytes!("datos/v2.json");
const V2_FIRMA: &[u8] = include_bytes!("datos/v2.json.minisig");
const ANTIGUO: &[u8] = include_bytes!("datos/antiguo.json");
const ANTIGUO_FIRMA: &[u8] = include_bytes!("datos/antiguo.json.minisig");

/// El modelo de los catálogos de prueba (su sha256 está firmado en ellos: si cambia este archivo, cambian).
fn modelo() -> Vec<u8> {
    gguf(
        &[
            ("general.architecture", "llama"),
            ("tokenizer.chat_template", "{{ x }}"),
        ],
        &[],
        300_000,
    )
}

fn repo(nombre: &str, archivo: &str) -> Repo {
    Repo {
        nombre: nombre.into(),
        archivos: vec![Archivo {
            ruta: archivo.into(),
            datos: modelo(),
            con_huella: true,
        }],
        licencia: Some("mit".into()),
        protegido: false,
    }
}

fn hf() -> comun::HfFalso {
    hf_falso(vec![
        repo("prueba/uno", "uno-Q4_0.gguf"),
        repo("prueba/falso", "falso-Q4_0.gguf"),
        repo("prueba/dos", "dos-Q4_0.gguf"),
    ])
}

#[test]
fn al_abrir_usa_el_catalogo_que_lleva_locall() {
    let carpeta = CarpetaDePrueba::nueva("cat-incluido");
    let nucleo = nucleo(&carpeta, None);
    let estado = nucleo.catalogo();
    assert_eq!(estado.origen, OrigenCatalogo::Incluido);
    assert_eq!(estado.comprobado, None);
    assert!(estado.catalogo.resolver("qwen3-4b").is_some());
}

#[test]
fn actualiza_una_vez_al_dia_y_lo_guarda() {
    let hf = hf();
    let carpeta = CarpetaDePrueba::nueva("cat-actualizar");
    let nucleo = nucleo_con_catalogo(&carpeta, &hf);
    servir_catalogo(&hf, V1, V1_FIRMA);

    assert_eq!(
        nucleo.actualizar_catalogo(false).unwrap(),
        Actualizacion::Actualizado
    );
    let estado = nucleo.catalogo();
    assert_eq!(estado.origen, OrigenCatalogo::Descargado);
    assert_eq!(estado.catalogo.publicado, 1_900_000_000);
    assert!(estado.comprobado.is_some());

    // Hasta mañana no vuelve a mirar (ni siquiera pide nada a GitHub).
    let antes = hf.pedidas.lock().unwrap().len();
    assert_eq!(
        nucleo.actualizar_catalogo(false).unwrap(),
        Actualizacion::NoTocaba
    );
    assert_eq!(hf.pedidas.lock().unwrap().len(), antes);
    // Forzando, sí; y como es el mismo, no hay novedades.
    assert_eq!(
        nucleo.actualizar_catalogo(true).unwrap(),
        Actualizacion::SinNovedades
    );

    // Al volver a abrir, sigue usando el descargado (está guardado con su firma) y recuerda cuándo miró.
    drop(nucleo);
    let otra_vez = nucleo_con_catalogo(&carpeta, &hf);
    let estado = otra_vez.catalogo();
    assert_eq!(estado.origen, OrigenCatalogo::Descargado);
    assert_eq!(estado.catalogo.publicado, 1_900_000_000);
    assert_eq!(
        otra_vez.actualizar_catalogo(false).unwrap(),
        Actualizacion::NoTocaba
    );

    // Uno más nuevo, sí.
    servir_catalogo(&hf, V2, V2_FIRMA);
    assert_eq!(
        otra_vez.actualizar_catalogo(true).unwrap(),
        Actualizacion::Actualizado
    );
    assert!(otra_vez.catalogo().catalogo.resolver("prueba-2b").is_some());
}

#[test]
fn nunca_acepta_un_catalogo_mas_antiguo() {
    let hf = hf();
    let carpeta = CarpetaDePrueba::nueva("cat-antiguo");
    let nucleo = nucleo_con_catalogo(&carpeta, &hf);
    servir_catalogo(&hf, V1, V1_FIRMA);
    nucleo.actualizar_catalogo(true).unwrap();

    // Bien firmado, pero más antiguo que el que hay: se ignora.
    servir_catalogo(&hf, ANTIGUO, ANTIGUO_FIRMA);
    assert_eq!(
        nucleo.actualizar_catalogo(true).unwrap(),
        Actualizacion::SinNovedades
    );
    let estado = nucleo.catalogo();
    assert_eq!(estado.catalogo.publicado, 1_900_000_000);
    assert!(estado.catalogo.resolver("viejo-1b").is_none());
}

#[test]
fn rechaza_un_catalogo_con_la_firma_mal() {
    let hf = hf();
    let carpeta = CarpetaDePrueba::nueva("cat-firma");
    let nucleo = nucleo_con_catalogo(&carpeta, &hf);
    // El contenido de v2 con la firma de v1: como si alguien lo hubiera cambiado por el camino.
    servir_catalogo(&hf, V2, V1_FIRMA);
    assert!(matches!(
        nucleo.actualizar_catalogo(true),
        Err(ErrorNucleo::Catalogo(ErrorCatalogo::FirmaIncorrecta))
    ));
    assert_eq!(
        nucleo.catalogo().origen,
        OrigenCatalogo::Incluido,
        "sigue con el que había"
    );
    assert!(
        !carpeta.0.join("catalogo").join("catalogo.json").exists(),
        "no se guarda"
    );
}

#[test]
fn si_github_no_lo_tiene_se_explica() {
    let hf = hf();
    let carpeta = CarpetaDePrueba::nueva("cat-404");
    let nucleo = nucleo_con_catalogo(&carpeta, &hf);
    // No se sirve nada: el Hugging Face falso responde 401 a lo que no conoce.
    assert!(matches!(
        nucleo.actualizar_catalogo(true),
        Err(ErrorNucleo::CatalogoNoDisponible(_))
    ));
    assert_eq!(nucleo.catalogo().origen, OrigenCatalogo::Incluido);
}

#[test]
fn instala_por_nombre_comprobando_lo_firmado() {
    let hf = hf();
    let carpeta = CarpetaDePrueba::nueva("cat-instalar");
    let nucleo = nucleo_con_catalogo(&carpeta, &hf);
    servir_catalogo(&hf, V1, V1_FIRMA);
    nucleo.actualizar_catalogo(true).unwrap();
    let nada = AtomicBool::new(false);

    // Por nombre (sin distinguir mayúsculas): el identificador es el nombre del catálogo.
    let m = nucleo
        .instalar_del_catalogo(" Prueba-1B ", &nada, |_| {})
        .unwrap();
    assert_eq!(m.id.to_string(), "prueba-1b");
    assert_eq!(m.revision, comun::COMMIT, "fijado al commit del catálogo");

    // Hugging Face tiene el archivo, pero con otra huella que la firmada: no se descarga nada.
    let error = nucleo
        .instalar_del_catalogo("falso-1b", &nada, |_| {})
        .unwrap_err();
    assert!(
        matches!(error, ErrorNucleo::CatalogoNoCoincide(_)),
        "{error}"
    );
    assert!(
        !hf.pedidas
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.contains("/resolve/") && p.contains("falso")),
        "no se pidió el archivo"
    );

    let error = nucleo
        .instalar_del_catalogo("no-existe", &nada, |_| {})
        .unwrap_err();
    assert!(matches!(error, ErrorNucleo::NoEnCatalogo(_)), "{error}");
}
