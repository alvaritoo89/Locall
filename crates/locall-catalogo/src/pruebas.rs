//! Validación del contenido (sin firma: la firma se prueba en `tests/firma.rs`).

use serde_json::{Value, json};

use crate::{ErrorCatalogo, FORMATO, leer};

/// Un catálogo válido mínimo, para estropearlo campo a campo.
fn base() -> Value {
    json!({
        "format": 1,
        "published": 1794300000u64,
        "models": [{
            "name": "qwen3-4b",
            "title": "Qwen3 4B",
            "publisher": "Qwen",
            "description": "Conversa y programa.",
            "tags": ["conversar", "programar"],
            "license": "apache-2.0",
            "context_length": 40960,
            "variants": [
                {
                    "name": "q4_k_m", "default": true, "repo": "Qwen/Qwen3-4B-GGUF",
                    "commit": "0123456789abcdef0123456789abcdef01234567", "checked_engine": "b12189",
                    "files": [{"path": "Qwen3-4B-Q4_K_M.gguf", "size": 2497280256u64,
                               "sha256": "047bf46455a544931cff6fef14d7910154c56afbc23ab1c5e56a72e69912c04b"}]
                },
                {
                    "name": "q8_0", "repo": "Qwen/Qwen3-4B-GGUF",
                    "commit": "0123456789abcdef0123456789abcdef01234567", "checked_engine": "b12189",
                    "files": [{"path": "Qwen3-4B-Q8_0.gguf", "size": 4280000000u64,
                               "sha256": "6151b19256b2bb93bc4b9d1fe6f17c14bd5be8f42bb0f6f6fcbe1c9d9a6f3a04"}]
                }
            ]
        }]
    })
}

fn leer_json(v: &Value) -> Result<crate::Catalogo, ErrorCatalogo> {
    leer(v.to_string().as_bytes())
}

/// Aplica un cambio a la base y devuelve el mensaje de error.
fn error_con(cambio: impl FnOnce(&mut Value)) -> String {
    let mut v = base();
    cambio(&mut v);
    leer_json(&v).unwrap_err().to_string()
}

#[test]
fn lee_un_catalogo_valido() {
    let c = leer_json(&base()).unwrap();
    let m = &c.modelos[0];
    assert_eq!(m.nombre, "qwen3-4b");
    assert_eq!(m.etiquetas, ["conversar", "programar"]);
    assert_eq!(m.por_defecto().nombre, "q4_k_m");
    assert_eq!(m.variantes[1].bytes(), 4_280_000_000);
    assert!(!m.variantes[1].por_defecto, "sin «default» es false");
}

#[test]
fn resuelve_el_nombre_corto_y_la_variante() {
    let c = leer_json(&base()).unwrap();
    let (m, v) = c.resolver("qwen3-4b").unwrap();
    assert_eq!(
        (m.nombre.as_str(), v.nombre.as_str()),
        ("qwen3-4b", "q4_k_m")
    );
    let (_, v) = c.resolver("  Qwen3-4B:Q8_0 ").unwrap();
    assert_eq!(v.nombre, "q8_0", "sin distinguir mayúsculas ni espacios");
    assert!(
        c.resolver("qwen3-4b:q2_k").is_none(),
        "variante que no existe"
    );
    assert!(c.resolver("llama").is_none());
}

#[test]
fn un_formato_mas_nuevo_pide_actualizar() {
    let mensaje = error_con(|v| v["format"] = json!(FORMATO + 1));
    assert!(mensaje.contains("actualiza Locall"), "{mensaje}");
}

/// Un cambio que estropea la base y una pista de lo que debe decir el error.
type Caso = (Box<dyn FnOnce(&mut Value)>, &'static str);

#[test]
fn cada_campo_no_valido_se_explica() {
    let casos: Vec<Caso> = vec![
        (Box::new(|v| v["published"] = json!(0)), "«published»"),
        (Box::new(|v| v["models"] = json!("x")), "«models»"),
        (
            Box::new(|v| v["models"][0]["name"] = json!("Qwen 3")),
            "«name»",
        ),
        (
            Box::new(|v| v["models"][0]["name"] = json!("-qwen")),
            "«name»",
        ),
        (
            Box::new(|v| v["models"][0]["title"] = json!("")),
            "«title» está vacío",
        ),
        (
            Box::new(|v| v["models"][0]["description"] = json!("a".repeat(2001))),
            "demasiado largo",
        ),
        (
            Box::new(|v| v["models"][0]["description"] = json!("hola\u{7}")),
            "caracteres de control",
        ),
        (
            Box::new(|v| v["models"][0]["tags"] = json!(["Conversar"])),
            "etiqueta",
        ),
        (
            Box::new(|v| v["models"][0]["context_length"] = json!(-1)),
            "«context_length»",
        ),
        (
            Box::new(|v| v["models"][0]["variants"] = json!([])),
            "ninguna variante",
        ),
        (
            Box::new(|v| v["models"][0]["variants"][1]["default"] = json!(true)),
            "exactamente una",
        ),
        (
            Box::new(|v| v["models"][0]["variants"][0]["default"] = json!(false)),
            "exactamente una",
        ),
        (
            Box::new(|v| v["models"][0]["variants"][0]["default"] = json!("sí")),
            "«default»",
        ),
        (
            Box::new(|v| v["models"][0]["variants"][1]["name"] = json!("q4_k_m")),
            "repetida",
        ),
        (
            Box::new(|v| v["models"][0]["variants"][0]["name"] = json!("Q4-K")),
            "el nombre solo puede",
        ),
        (
            Box::new(|v| v["models"][0]["variants"][0]["repo"] = json!("sin-barra")),
            "«repo»",
        ),
        (
            Box::new(|v| v["models"][0]["variants"][0]["commit"] = json!("main")),
            "«commit»",
        ),
        (
            Box::new(|v| {
                v["models"][0]["variants"][0]["commit"] =
                    json!("0123456789ABCDEF0123456789abcdef01234567")
            }),
            "«commit»",
        ),
        (
            Box::new(|v| v["models"][0]["variants"][0]["files"] = json!([])),
            "no tiene archivos",
        ),
        (
            Box::new(|v| {
                v["models"][0]["variants"][0]["files"][0]["path"] = json!("../../malo.gguf")
            }),
            "ruta válida",
        ),
        (
            Box::new(|v| v["models"][0]["variants"][0]["files"][0]["path"] = json!("/abs.gguf")),
            "ruta válida",
        ),
        (
            Box::new(|v| v["models"][0]["variants"][0]["files"][0]["path"] = json!("a\\b.gguf")),
            "ruta válida",
        ),
        (
            Box::new(|v| v["models"][0]["variants"][0]["files"][0]["path"] = json!("modelo.exe")),
            "ruta válida",
        ),
        (
            Box::new(|v| v["models"][0]["variants"][0]["files"][0]["size"] = json!(0)),
            "tamaño 0",
        ),
        (
            Box::new(|v| v["models"][0]["variants"][0]["files"][0]["sha256"] = json!("abc")),
            "«sha256»",
        ),
        (
            Box::new(|v| {
                let m = v["models"][0].clone();
                v["models"].as_array_mut().unwrap().push(m);
            }),
            "repetido",
        ),
    ];
    for (i, (cambio, pista)) in casos.into_iter().enumerate() {
        let mensaje = error_con(cambio);
        assert!(
            mensaje.contains(pista),
            "caso {i}: «{mensaje}» debería mencionar «{pista}»"
        );
    }
}

#[test]
fn un_json_roto_se_explica() {
    let error = leer(b"{ esto no es json").unwrap_err();
    assert!(matches!(error, ErrorCatalogo::Json(_)), "{error}");
}

#[test]
fn las_listas_tienen_un_maximo() {
    let mensaje = error_con(|v| {
        let modelo = v["models"][0].clone();
        let lista: Vec<Value> = (0..501)
            .map(|i| {
                let mut m = modelo.clone();
                m["name"] = json!(format!("m{i}"));
                m
            })
            .collect();
        v["models"] = json!(lista);
    });
    assert!(mensaje.contains("demasiados"), "{mensaje}");
}
