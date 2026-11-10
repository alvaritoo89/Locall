//! `GET` y `PATCH /api/settings`: ver los ajustes y cambiarlos (se aplican al volver a abrir Locall).

mod comun;

use comun::arrancar;
use comun::hf::hf_falso;
use serde_json::json;

#[test]
fn cambiar_ajustes_los_guarda_y_avisa_de_que_hay_que_reiniciar() {
    let hf = hf_falso(vec![]);
    let api = arrancar("ajustes", &hf);

    let (estado, a) = api.json("GET", "/api/settings", None);
    assert_eq!(estado, 200, "{a}");
    assert_eq!(a["active"], a["saved"]);
    assert_eq!(a["restart_needed"], false);
    assert_eq!(a["active"]["context"], 4096);
    assert_eq!(a["active"]["threads"], "auto");
    assert!(a["threads_available"].as_u64().unwrap() >= 1);

    let (estado, a) = api.json(
        "PATCH",
        "/api/settings",
        Some(&json!({"context": 8192, "unload_minutes": 0, "threads": 1})),
    );
    assert_eq!(estado, 200, "{a}");
    assert_eq!(a["saved"]["context"], 8192);
    assert_eq!(a["saved"]["unload_minutes"], 0);
    assert_eq!(a["saved"]["threads"], 1);
    assert_eq!(
        a["active"]["context"], 4096,
        "lo que está en uso no cambia hasta reiniciar"
    );
    assert_eq!(a["restart_needed"], true);

    let texto = std::fs::read_to_string(api.carpeta.join("config.toml")).unwrap();
    assert!(texto.contains("contexto = 8192"), "{texto}");
    assert!(
        texto.contains("# Contexto por defecto"),
        "los comentarios siguen ahí"
    );

    // Volver a los valores en uso: ya no hace falta reiniciar.
    let (_, a) = api.json(
        "PATCH",
        "/api/settings",
        Some(&json!({"context": 4096, "unload_minutes": 5, "threads": "auto"})),
    );
    assert_eq!(a["restart_needed"], false, "{a}");
}

#[test]
fn rechaza_cambios_no_validos_sin_tocar_el_archivo() {
    let hf = hf_falso(vec![]);
    let api = arrancar("ajustes-mal", &hf);
    let _ = api.json("PATCH", "/api/settings", Some(&json!({"log_days": 10})));
    let antes = std::fs::read_to_string(api.carpeta.join("config.toml")).unwrap();

    for (cuerpo, pista) in [
        (json!({}), "objeto"),
        (json!({"port": 12000}), "«port»"),
        (json!({"models_folder": "D:\\x"}), "«models_folder»"),
        (json!({"context": "mucho"}), "número"),
        (json!({"context": 10}), "contexto"),
        (json!({"threads": 100000}), "hilos"),
        (json!({"threads": "todos"}), "número"),
        (json!({"log_days": 0}), "registros_dias"),
    ] {
        let (estado, r) = api.json("PATCH", "/api/settings", Some(&cuerpo));
        assert_eq!(estado, 400, "{cuerpo}: {r}");
        assert_eq!(r["error"]["code"], "invalid_request");
        let mensaje = r["error"]["message"].as_str().unwrap();
        assert!(
            mensaje.contains(pista),
            "{cuerpo}: {mensaje:?} debería mencionar {pista}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(api.carpeta.join("config.toml")).unwrap(),
        antes
    );
}
