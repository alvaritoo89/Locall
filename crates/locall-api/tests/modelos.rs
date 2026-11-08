//! Gestión de modelos por la API, contra un Hugging Face falso (sin internet).

mod comun;

use std::io::{BufRead, BufReader};
use std::time::{Duration, Instant};

use comun::hf::{Archivo, Repo, gguf, hf_falso};
use comun::{Api, arrancar};
use serde_json::{Value, json};

fn modelo_llama(relleno: usize) -> Vec<u8> {
    gguf(&[("general.architecture", "llama")], &[], relleno)
}

fn repo(nombre: &str, archivos: Vec<(&str, Vec<u8>)>, lento: bool) -> Repo {
    Repo {
        nombre: nombre.into(),
        archivos: archivos
            .into_iter()
            .map(|(ruta, datos)| Archivo {
                ruta: ruta.into(),
                datos,
                con_huella: true,
            })
            .collect(),
        licencia: Some("apache-2.0".into()),
        protegido: false,
        lento,
    }
}

/// Lee una respuesta NDJSON entera.
fn eventos(respuesta: ureq::http::Response<ureq::Body>) -> Vec<Value> {
    BufReader::new(respuesta.into_body().into_reader())
        .lines()
        .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
        .collect()
}

fn instalar(api: &Api, repo: &str, archivo: &str) -> Vec<Value> {
    let respuesta = api.pedir(
        "POST",
        "/api/models/pull",
        Some(&json!({"repo": repo, "file": archivo})),
    );
    assert_eq!(respuesta.status().as_u16(), 200);
    eventos(respuesta)
}

#[test]
fn consulta_un_repositorio() {
    let hf = hf_falso(vec![repo(
        "prueba/varios",
        vec![
            ("a-Q4_0.gguf", modelo_llama(10)),
            ("README.md", b"x".to_vec()),
        ],
        false,
    )]);
    let api = arrancar("hub", &hf);
    let (estado, json) = api.json("GET", "/api/hub/prueba/varios?revision=main", None);
    assert_eq!(estado, 200);
    assert_eq!(json["commit"], comun::hf::COMMIT);
    assert_eq!(json["license"], "apache-2.0");
    let archivos = json["files"].as_array().unwrap();
    assert_eq!(archivos.len(), 1, "solo los GGUF");
    assert_eq!(archivos[0]["path"], "a-Q4_0.gguf");
    assert_eq!(archivos[0]["sha256"].as_str().unwrap().len(), 64);

    let (estado, json) = api.json("GET", "/api/hub/prueba/no-existe", None);
    assert_eq!(
        (estado, json["error"]["code"].as_str()),
        (404, Some("not_found_on_hub"))
    );
}

#[test]
fn instala_con_progreso_y_aparece_en_la_lista() {
    let hf = hf_falso(vec![repo(
        "prueba/m",
        vec![("Modelo-Q4_K_M.gguf", modelo_llama(1_000_000))],
        false,
    )]);
    let api = arrancar("pull", &hf);

    let eventos = instalar(&api, "prueba/m", "Modelo-Q4_K_M.gguf");
    let progreso: Vec<&Value> = eventos.iter().filter(|e| e["type"] == "progress").collect();
    assert!(!progreso.is_empty());
    let ultimo_progreso = progreso.last().unwrap();
    assert_eq!(
        ultimo_progreso["downloaded_bytes"],
        ultimo_progreso["total_bytes"]
    );
    let fin = eventos.last().unwrap();
    assert_eq!(fin["type"], "done");
    assert_eq!(fin["model"]["id"], "modelo-q4_k_m");
    assert_eq!(fin["model"]["revision"], comun::hf::COMMIT);

    let (_, lista) = api.json("GET", "/api/models", None);
    assert_eq!(lista["models"][0]["id"], "modelo-q4_k_m");

    // Instalarlo otra vez: conflicto, antes de empezar (error HTTP normal).
    let (estado, json) = api.json(
        "POST",
        "/api/models/pull",
        Some(&json!({"repo": "prueba/m", "file": "Modelo-Q4_K_M.gguf"})),
    );
    assert_eq!(
        (estado, json["error"]["code"].as_str()),
        (409, Some("model_exists"))
    );
}

#[test]
fn sin_trozos_responde_el_modelo_al_final() {
    let hf = hf_falso(vec![repo(
        "prueba/m",
        vec![("m.gguf", modelo_llama(1000))],
        false,
    )]);
    let api = arrancar("pull-entero", &hf);
    let (estado, json) = api.json(
        "POST",
        "/api/models/pull",
        Some(&json!({"repo": "prueba/m", "file": "m.gguf", "id": "mio", "stream": false})),
    );
    assert_eq!(estado, 200);
    assert_eq!(json["id"], "mio");
}

#[test]
fn explica_lo_que_no_se_puede_instalar() {
    let hf = hf_falso(vec![repo(
        "prueba/raro",
        vec![(
            "raro.gguf",
            gguf(&[("general.architecture", "inventada")], &[], 100),
        )],
        false,
    )]);
    let api = arrancar("pull-errores", &hf);
    let casos = [
        (json!({"file": "x.gguf"}), 400, "invalid_request"),
        (json!({"repo": "prueba/raro"}), 400, "invalid_request"),
        (
            json!({"repo": "sin-barra", "file": "x.gguf"}),
            400,
            "invalid_request",
        ),
        (
            json!({"repo": "prueba/no-existe", "file": "x.gguf"}),
            404,
            "not_found_on_hub",
        ),
        (
            json!({"repo": "prueba/raro", "file": "otro.gguf"}),
            404,
            "not_found_on_hub",
        ),
        (
            json!({"repo": "prueba/raro", "file": "raro.gguf"}),
            422,
            "model_incompatible",
        ),
    ];
    for (cuerpo, estado, codigo) in casos {
        let (e, json) = api.json("POST", "/api/models/pull", Some(&cuerpo));
        assert_eq!(
            (e, json["error"]["code"].as_str()),
            (estado, Some(codigo)),
            "{cuerpo}"
        );
    }
}

#[test]
fn si_el_cliente_se_va_la_descarga_se_pausa_y_luego_continua() {
    // 3 MB a 64 KB cada 20 ms ≈ 1 s de descarga.
    let hf = hf_falso(vec![repo(
        "prueba/lento",
        vec![("l.gguf", modelo_llama(3_000_000))],
        true,
    )]);
    let api = arrancar("pull-pausa", &hf);

    // 1) Empezar y colgar tras el primer aviso de progreso con datos.
    let respuesta = api.pedir(
        "POST",
        "/api/models/pull",
        Some(&json!({"repo": "prueba/lento", "file": "l.gguf"})),
    );
    let mut lector = BufReader::new(respuesta.into_body().into_reader());
    let mut linea = String::new();
    loop {
        linea.clear();
        lector.read_line(&mut linea).unwrap();
        let e: Value = serde_json::from_str(&linea).unwrap();
        if e["downloaded_bytes"].as_u64().unwrap_or(0) > 100_000 {
            break;
        }
    }
    drop(lector);

    // 2) Volver a pedirlo. Mientras el núcleo se entera de que el primero se fue, puede decir "ya se está
    //    instalando": se reintenta hasta 3 s.
    let inicio = Instant::now();
    let eventos = loop {
        let respuesta = api.pedir(
            "POST",
            "/api/models/pull",
            Some(&json!({"repo": "prueba/lento", "file": "l.gguf"})),
        );
        if respuesta.status().as_u16() == 200 {
            break eventos(respuesta);
        }
        assert_eq!(respuesta.status().as_u16(), 409);
        assert!(
            inicio.elapsed() < Duration::from_secs(3),
            "la primera instalación no se canceló"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    // Continuó desde lo ya descargado, no desde cero.
    let primero = eventos.iter().find(|e| e["type"] == "progress").unwrap();
    assert!(
        primero["downloaded_bytes"].as_u64().unwrap() > 100_000,
        "empezó de cero: {primero}"
    );
    assert_eq!(eventos.last().unwrap()["type"], "done");
}

#[test]
fn importa_y_elimina() {
    let hf = hf_falso(vec![]);
    let api = arrancar("importar", &hf);
    let archivo = api.carpeta.join("Local-Q8_0.gguf");
    std::fs::write(&archivo, modelo_llama(5000)).unwrap();

    let (estado, json) = api.json(
        "POST",
        "/api/models/import",
        Some(&json!({"path": archivo.to_string_lossy()})),
    );
    assert_eq!(estado, 200, "{json}");
    assert_eq!(json["id"], "local-q8_0");
    assert_eq!(json["source"], "local");

    let (estado, json) = api.json(
        "POST",
        "/api/models/import",
        Some(&json!({"path": "relativa.gguf"})),
    );
    assert_eq!(
        (estado, json["error"]["code"].as_str()),
        (400, Some("invalid_request"))
    );
    let (estado, json) = api.json(
        "POST",
        "/api/models/import",
        Some(&json!({"path": api.carpeta.join("no-esta.gguf").to_string_lossy()})),
    );
    assert_eq!(
        (estado, json["error"]["code"].as_str()),
        (404, Some("file_not_found"))
    );

    let (estado, json) = api.json("DELETE", "/api/models/local-q8_0", None);
    assert_eq!(estado, 200);
    assert_eq!(json["deleted"], "local-q8_0");
    assert!(
        archivo.is_file(),
        "el archivo original del usuario sigue ahí"
    );

    let (estado, json) = api.json("DELETE", "/api/models/local-q8_0", None);
    assert_eq!(
        (estado, json["error"]["code"].as_str()),
        (404, Some("model_not_found"))
    );
}
