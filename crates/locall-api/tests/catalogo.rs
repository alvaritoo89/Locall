//! El catálogo por la API (sin internet: el catálogo de GitHub se busca en el servidor falso, que no lo tiene).

mod comun;

use comun::arrancar;
use comun::hf::hf_falso;
use serde_json::json;

#[test]
fn muestra_el_catalogo_que_lleva_locall() {
    let hf = hf_falso(vec![]);
    let api = arrancar("cat-ver", &hf);
    let (estado, c) = api.json("GET", "/api/catalog", None);
    assert_eq!(estado, 200, "{c}");
    assert_eq!(c["source"], "builtin");
    assert_eq!(c["checked_at"], json!(null));
    let modelos = c["models"].as_array().unwrap();
    assert_eq!(modelos.len(), 12);
    let qwen = modelos.iter().find(|m| m["name"] == "qwen3-4b").unwrap();
    assert_eq!(qwen["status"], "compatible", "nunca «probado» (ADR 0022)");
    assert_eq!(qwen["installed"], false);
    assert_eq!(qwen["repo"], "Qwen/Qwen3-4B-GGUF");
    assert!(qwen["size_bytes"].as_u64().unwrap() > 2_000_000_000);
    assert!(qwen["memory_estimate_bytes"].as_u64().unwrap() > qwen["size_bytes"].as_u64().unwrap());
    assert!(
        ["comfortable", "fits", "tight", "too_big"]
            .contains(&qwen["fit_estimate"].as_str().unwrap())
    );
}

#[test]
fn si_no_hay_catalogo_nuevo_lo_explica_y_sigue_con_el_suyo() {
    let hf = hf_falso(vec![]);
    let api = arrancar("cat-actualizar", &hf);
    let (estado, r) = api.json("POST", "/api/catalog/update", Some(&json!({"force": true})));
    assert_eq!(estado, 502, "{r}");
    assert_eq!(r["error"]["code"], "catalog_unavailable");
    // Ya miró: sin forzar, no vuelve a intentarlo hasta mañana.
    let (estado, r) = api.json("POST", "/api/catalog/update", None);
    assert_eq!(estado, 200, "{r}");
    assert_eq!(r["result"], "not_due");
    assert_eq!(r["catalog"]["source"], "builtin");
    assert!(r["catalog"]["checked_at"].as_u64().is_some());

    let (estado, r) = api.json("POST", "/api/catalog/update", Some(&json!({"force": "sí"})));
    assert_eq!(estado, 400, "{r}");
}

#[test]
fn instalar_por_nombre_valida_la_peticion() {
    let hf = hf_falso(vec![]);
    let api = arrancar("cat-instalar", &hf);
    let (estado, r) = api.json(
        "POST",
        "/api/models/pull",
        Some(&json!({"name": "no-existe"})),
    );
    assert_eq!(estado, 404, "{r}");
    assert_eq!(r["error"]["code"], "not_in_catalog");
    let (estado, r) = api.json(
        "POST",
        "/api/models/pull",
        Some(&json!({"name": "qwen3-4b", "repo": "Qwen/Qwen3-4B-GGUF"})),
    );
    assert_eq!(estado, 400, "{r}");
    assert!(
        r["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no los dos")
    );
}
