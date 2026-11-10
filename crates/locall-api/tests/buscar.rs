//! `POST /api/hub/search`: buscar repositorios por nombre, contra un Hugging Face falso (sin internet).

mod comun;

use comun::arrancar;
use comun::hf::{Archivo, Repo, hf_falso};
use serde_json::json;

fn repo(nombre: &str, archivos: usize, licencia: Option<&str>) -> Repo {
    Repo {
        nombre: nombre.into(),
        archivos: (0..archivos)
            .map(|i| Archivo {
                ruta: format!("m-{i}.gguf"),
                datos: vec![0; 8],
                con_huella: true,
            })
            .collect(),
        licencia: licencia.map(Into::into),
        protegido: false,
        lento: false,
    }
}

#[test]
fn busca_por_nombre_y_ordena_por_descargas() {
    let hf = hf_falso(vec![
        repo("prueba/Qwen-Pequeno-GGUF", 1, Some("apache-2.0")),
        repo("otro/qwen-grande-GGUF", 3, None),
        repo("prueba/Llama-GGUF", 2, Some("llama3")),
    ]);
    let api = arrancar("buscar", &hf);

    let (estado, r) = api.json(
        "POST",
        "/api/hub/search",
        Some(&json!({"query": "  qwen  "})),
    );
    assert_eq!(estado, 200, "{r}");
    let resultados = r["results"].as_array().unwrap();
    assert_eq!(resultados.len(), 2);
    assert_eq!(
        resultados[0]["repo"], "otro/qwen-grande-GGUF",
        "el más descargado primero"
    );
    assert_eq!(resultados[0]["downloads"], 300);
    assert_eq!(resultados[0]["license"], json!(null));
    assert_eq!(resultados[1]["license"], "apache-2.0");
    assert_eq!(resultados[1]["task"], "text-generation");

    // A Hugging Face solo le llega el texto (sin espacios de más, codificado) y que se piden GGUF.
    let pedida = hf
        .pedidas
        .lock()
        .unwrap()
        .iter()
        .find(|p| p.starts_with("/api/models?"))
        .cloned()
        .unwrap();
    assert!(
        pedida.starts_with("/api/models?search=qwen&filter=gguf&sort=downloads"),
        "{pedida}"
    );
    assert!(pedida.ends_with("&limit=20"), "{pedida}");

    let (_, r) = api.json(
        "POST",
        "/api/hub/search",
        Some(&json!({"query": "no hay nada así"})),
    );
    assert_eq!(r["results"], json!([]));
}

#[test]
fn rechaza_busquedas_mal_formadas() {
    let hf = hf_falso(vec![]);
    let api = arrancar("buscar-mal", &hf);
    for cuerpo in [
        json!({}),
        json!({"query": "   "}),
        json!({"query": "a".repeat(101)}),
        json!({"query": "dos\nlíneas"}),
        json!({"query": "qwen", "limit": 0}),
        json!({"query": "qwen", "limit": 51}),
    ] {
        let (estado, r) = api.json("POST", "/api/hub/search", Some(&cuerpo));
        assert_eq!(estado, 400, "{cuerpo}: {r}");
        assert_eq!(r["error"]["code"], "invalid_request");
    }
    assert!(
        hf.pedidas.lock().unwrap().is_empty(),
        "una búsqueda no válida no llega a Hugging Face"
    );
}
