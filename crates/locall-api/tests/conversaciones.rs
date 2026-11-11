//! Saved conversations through the API, without an engine: create, list, open, rename, delete and validation.
//! (Writing to a conversation with the real engine is tested in `chat.rs`.)

mod comun;

use comun::arrancar;
use comun::hf::hf_falso;
use serde_json::json;

#[test]
fn crear_listar_abrir_renombrar_y_eliminar() {
    let hf = hf_falso(vec![]);
    let api = arrancar("conv-crud", &hf);

    let (estado, lista) = api.json("GET", "/api/conversations", None);
    assert_eq!(estado, 200);
    assert_eq!(lista["conversations"], json!([]));

    // No body: default title.
    let (estado, primera) = api.json("POST", "/api/conversations", None);
    assert_eq!(estado, 201);
    assert_eq!(primera["title"], "Nueva conversación");
    assert_eq!(primera["message_count"], 0);
    let (_, segunda) = api.json(
        "POST",
        "/api/conversations",
        Some(&json!({"title": "Recetas"})),
    );
    assert_eq!(segunda["title"], "Recetas");

    let (_, lista) = api.json("GET", "/api/conversations", None);
    let ids: Vec<_> = lista["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&primera["id"].as_i64().unwrap()));

    let ruta = format!("/api/conversations/{}", segunda["id"]);
    let (estado, abierta) = api.json("GET", &ruta, None);
    assert_eq!(estado, 200);
    assert_eq!(abierta["title"], "Recetas");
    assert_eq!(abierta["messages"], json!([]));

    let (estado, renombrada) = api.json(
        "PATCH",
        &ruta,
        Some(&json!({"title": "  Postres\nde  verano "})),
    );
    assert_eq!(estado, 200);
    assert_eq!(renombrada["title"], "Postres de verano", "una sola línea");

    let (estado, borrada) = api.json("DELETE", &ruta, None);
    assert_eq!(estado, 200);
    assert_eq!(borrada["deleted"], segunda["id"]);
    let (estado, error) = api.json("GET", &ruta, None);
    assert_eq!(estado, 404);
    assert_eq!(error["error"]["code"], "conversation_not_found");
    let (_, lista) = api.json("GET", "/api/conversations", None);
    assert_eq!(lista["conversations"].as_array().unwrap().len(), 1);
}

#[test]
fn errores_claros() {
    let hf = hf_falso(vec![]);
    let api = arrancar("conv-errores", &hf);
    let (_, c) = api.json("POST", "/api/conversations", None);
    let ruta = format!("/api/conversations/{}", c["id"]);
    let mensajes = format!("{ruta}/messages");

    // What does not exist: 404 with the right code.
    for (metodo, ruta, cuerpo) in [
        ("GET", "/api/conversations/999", None),
        ("DELETE", "/api/conversations/999", None),
        (
            "PATCH",
            "/api/conversations/999",
            Some(json!({"title": "x"})),
        ),
        (
            "POST",
            "/api/conversations/999/messages",
            Some(json!({"content": "hola", "model": "x"})),
        ),
    ] {
        let (estado, error) = api.json(metodo, ruta, cuerpo.as_ref());
        assert_eq!(estado, 404, "{metodo} {ruta}");
        assert_eq!(
            error["error"]["code"], "conversation_not_found",
            "{metodo} {ruta}"
        );
    }
    let (estado, error) = api.json(
        "POST",
        &mensajes,
        Some(&json!({"content": "hola", "model": "no-existe"})),
    );
    assert_eq!(estado, 404);
    assert_eq!(error["error"]["code"], "model_not_found");
    let (_, abierta) = api.json("GET", &ruta, None);
    assert_eq!(
        abierta["message_count"], 0,
        "una pregunta a un modelo que no existe no se guarda"
    );
    assert_eq!(abierta["model"], json!(null));

    // Malformed requests: 400.
    let casos = [
        (
            "GET",
            "/api/conversations/abc".to_owned(),
            None,
            "identificador",
        ),
        (
            "GET",
            "/api/conversations/0".to_owned(),
            None,
            "identificador",
        ),
        (
            "POST",
            "/api/conversations".to_owned(),
            Some(json!({"title": 3})),
            "«title»",
        ),
        ("PATCH", ruta.clone(), Some(json!({})), "«title»"),
        (
            "PATCH",
            ruta.clone(),
            Some(json!({"title": "   "})),
            "«title»",
        ),
        (
            "POST",
            mensajes.clone(),
            Some(json!({"model": "x"})),
            "«content»",
        ),
        (
            "POST",
            mensajes.clone(),
            Some(json!({"content": "  ", "model": "x"})),
            "«content»",
        ),
        (
            "POST",
            mensajes.clone(),
            Some(json!({"content": "hola"})),
            "«model»",
        ),
        (
            "POST",
            mensajes.clone(),
            Some(json!({"content": "hola", "model": "x", "system": 1})),
            "«system»",
        ),
        (
            "POST",
            mensajes.clone(),
            Some(json!({"content": "hola", "model": "x", "temperature": 5})),
            "«temperature»",
        ),
        ("POST", mensajes.clone(), Some(json!([1, 2])), "objeto"),
    ];
    for (metodo, ruta, cuerpo, pista) in casos {
        let (estado, error) = api.json(metodo, &ruta, cuerpo.as_ref());
        assert_eq!(estado, 400, "{metodo} {ruta} {cuerpo:?}");
        assert_eq!(error["error"]["code"], "invalid_request");
        let mensaje = error["error"]["message"].as_str().unwrap();
        assert!(
            mensaje.contains(pista),
            "{mensaje:?} debería mencionar {pista}"
        );
    }
}
