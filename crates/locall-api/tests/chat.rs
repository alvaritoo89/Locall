//! `POST /api/chat`: validación (sin motor) y conversación con el motor real (se salta si faltan los archivos).

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use locall_config::Rutas;
use locall_core::{Nucleo, OpcionesNucleo};
use serde_json::{Value, json};

const MOTOR: &str = r"C:\Locall-poc\motor\cpu\llama-server.exe";
const MODELO: &str = r"C:\Locall-poc\modelos\qwen2.5-1.5b-instruct-q4_k_m.gguf";

struct Prueba {
    base: String,
    token: String,
    carpeta: PathBuf,
}

impl Drop for Prueba {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.carpeta);
    }
}

/// Arranca la API. Con `real`, importa el modelo de la Fase 0 como «qwen» (o devuelve `None` si falta).
fn arrancar(nombre: &str, real: bool) -> Option<Prueba> {
    if real && !(PathBuf::from(MOTOR).is_file() && PathBuf::from(MODELO).is_file()) {
        eprintln!("(prueba saltada: faltan el motor o el modelo)");
        return None;
    }
    let carpeta = std::env::temp_dir().join(format!("locall-chat-{nombre}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&carpeta);
    std::fs::create_dir_all(&carpeta).unwrap();
    // Contexto pequeño, para poder probar el "no cabe".
    std::fs::write(carpeta.join("config.toml"), "contexto = 2048\n").unwrap();
    let rutas = Rutas::en(carpeta.clone());
    let nucleo = Nucleo::abrir(OpcionesNucleo::nuevas(rutas.clone(), MOTOR)).unwrap();
    if real {
        nucleo
            .importar(&PathBuf::from(MODELO), Some("qwen"))
            .unwrap();
    }
    let token = locall_api::token::leer_o_crear(&rutas.token()).unwrap();
    let escucha = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let puerto = escucha.local_addr().unwrap().port();
    escucha.set_nonblocking(true).unwrap();
    let rutas_api = locall_api::rutas(Arc::new(nucleo), token.clone(), puerto);
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let escucha = tokio::net::TcpListener::from_std(escucha).unwrap();
                axum::serve(escucha, rutas_api).await.unwrap();
            });
    });
    Some(Prueba {
        base: format!("http://127.0.0.1:{puerto}"),
        token,
        carpeta,
    })
}

fn agente() -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build(),
    )
}

impl Prueba {
    /// POST /api/chat con este cuerpo; devuelve la respuesta sin leer.
    fn chat(&self, cuerpo: &str) -> ureq::http::Response<ureq::Body> {
        agente()
            .post(&format!("{}/api/chat", self.base))
            .header("Authorization", &format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .send(cuerpo)
            .unwrap()
    }

    fn chat_json(&self, cuerpo: &Value) -> (u16, Value) {
        let respuesta = self.chat(&cuerpo.to_string());
        let estado = respuesta.status().as_u16();
        let texto = respuesta.into_body().read_to_string().unwrap();
        (estado, serde_json::from_str(&texto).unwrap_or(Value::Null))
    }

    /// Una petición con cuerpo JSON a cualquier ruta; devuelve la respuesta sin leer.
    fn enviar(&self, ruta: &str, cuerpo: &Value) -> ureq::http::Response<ureq::Body> {
        agente()
            .post(&format!("{}{ruta}", self.base))
            .header("Authorization", &format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .send(cuerpo.to_string())
            .unwrap()
    }

    fn leer(&self, ruta: &str) -> Value {
        let texto = agente()
            .get(&format!("{}{ruta}", self.base))
            .header("Authorization", &format!("Bearer {}", self.token))
            .call()
            .unwrap()
            .into_body()
            .read_to_string()
            .unwrap();
        serde_json::from_str(&texto).unwrap()
    }

    fn estado(&self) -> Value {
        let texto = agente()
            .get(&format!("{}/api/status", self.base))
            .header("Authorization", &format!("Bearer {}", self.token))
            .call()
            .unwrap()
            .into_body()
            .read_to_string()
            .unwrap();
        serde_json::from_str(&texto).unwrap()
    }
}

fn pregunta(texto: &str, max_tokens: u32, stream: bool) -> Value {
    json!({
        "model": "qwen",
        "messages": [{"role": "user", "content": texto}],
        "stream": stream,
        "options": {"max_tokens": max_tokens, "temperature": 0, "seed": 1},
    })
}

// ─── Sin motor ────────────────────────────────────────────────────────────────

#[test]
fn rechaza_peticiones_mal_formadas() {
    let api = arrancar("validar", false).unwrap();
    let casos = [
        ("esto no es json".to_owned(), "JSON válido"),
        (
            json!({"messages": [{"role": "user", "content": "x"}]}).to_string(),
            "falta «model»",
        ),
        (
            json!({"model": "x", "messages": []}).to_string(),
            "al menos un mensaje",
        ),
        (
            json!({"model": "x", "messages": [{"role": "jefe", "content": "x"}]}).to_string(),
            "«role»",
        ),
        (
            json!({"model": "x", "messages": [{"role": "user"}]}).to_string(),
            "«content»",
        ),
        (
            json!({"model": "x", "messages": [{"role": "user", "content": "x"}],
                   "options": {"temperature": 7}})
            .to_string(),
            "entre 0 y 2",
        ),
        (
            json!({"model": "x", "messages": [{"role": "user", "content": "x"}], "stream": "sí"})
                .to_string(),
            "«stream»",
        ),
    ];
    for (cuerpo, fragmento) in casos {
        let respuesta = api.chat(&cuerpo);
        assert_eq!(respuesta.status().as_u16(), 400, "{cuerpo}");
        let json: Value =
            serde_json::from_str(&respuesta.into_body().read_to_string().unwrap()).unwrap();
        assert_eq!(json["error"]["code"], "invalid_request");
        let mensaje = json["error"]["message"].as_str().unwrap();
        assert!(mensaje.contains(fragmento), "{cuerpo}: {mensaje}");
    }
}

#[test]
fn un_modelo_que_no_existe_es_un_404() {
    let api = arrancar("no-existe", false).unwrap();
    let (estado, json) = api.chat_json(&json!({
        "model": "inventado", "messages": [{"role": "user", "content": "hola"}]
    }));
    assert_eq!(
        (estado, json["error"]["code"].as_str()),
        (404, Some("model_not_found"))
    );
}

// ─── Con el motor real ────────────────────────────────────────────────────────

#[test]
fn responde_por_trozos_en_ndjson() {
    let Some(api) = arrancar("trozos", true) else {
        return;
    };
    let respuesta = api.chat(&pregunta("¿Capital de Francia? Una palabra.", 10, true).to_string());
    assert_eq!(respuesta.status().as_u16(), 200);
    assert!(
        respuesta.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("application/x-ndjson")
    );

    let eventos: Vec<Value> = BufReader::new(respuesta.into_body().into_reader())
        .lines()
        .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
        .collect();
    let texto: String = eventos
        .iter()
        .filter(|e| e["type"] == "text")
        .map(|e| e["text"].as_str().unwrap())
        .collect();
    eprintln!("respuesta: {texto:?}");
    assert!(texto.contains("Par"), "{texto}");
    assert!(eventos.iter().any(|e| e["type"] == "prompt_progress"));
    let ultimo = eventos.last().unwrap();
    assert_eq!(ultimo["type"], "done");
    assert!(ultimo["usage"]["completion_tokens"].as_u64().unwrap() > 0);
    // Ningún evento lleva la ruta del modelo (D4).
    assert!(!eventos.iter().any(|e| e.to_string().contains("Locall-poc")));
}

#[test]
fn responde_de_una_vez_sin_trozos() {
    let Some(api) = arrancar("entera", true) else {
        return;
    };
    let (estado, json) = api.chat_json(&pregunta(
        "Escribe una redacción larga sobre el mar.",
        5,
        false,
    ));
    assert_eq!(estado, 200);
    assert_eq!(json["message"]["role"], "assistant");
    assert!(!json["message"]["content"].as_str().unwrap().is_empty());
    assert_eq!(json["finish"], "length");
    assert_eq!(json["usage"]["completion_tokens"], 5);
}

#[test]
fn una_conversacion_demasiado_larga_se_explica() {
    let Some(api) = arrancar("contexto", true) else {
        return;
    };
    let enorme = "hola ".repeat(3000); // ≈3000 tokens en un contexto de 2048
    let (estado, json) = api.chat_json(&pregunta(&enorme, 5, true));
    assert_eq!(
        (estado, json["error"]["code"].as_str()),
        (400, Some("context_exceeded"))
    );
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no cabe")
    );
}

#[test]
fn si_el_cliente_se_va_se_deja_de_generar() {
    let Some(api) = arrancar("desconexion", true) else {
        return;
    };
    let respuesta =
        api.chat(&pregunta("Escribe una redacción muy larga sobre el mar.", 400, true).to_string());
    let mut lector = BufReader::new(respuesta.into_body().into_reader());
    // Leer unos trozos de texto…
    let mut trozos = 0;
    let mut linea = String::new();
    while trozos < 5 {
        linea.clear();
        lector.read_line(&mut linea).unwrap();
        if linea.contains("\"text\"") {
            trozos += 1;
        }
    }
    assert_eq!(api.estado()["in_use"], 1, "se está usando");
    // …y colgar (400 tokens habrían tardado más de 10 s).
    drop(lector);

    let inicio = Instant::now();
    while api.estado()["in_use"] != 0 {
        assert!(
            inicio.elapsed() < Duration::from_secs(3),
            "sigue generando tras irse el cliente"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        api.estado()["state"],
        "ready",
        "el modelo sigue cargado para la próxima"
    );
}

#[test]
fn una_conversacion_guardada_recuerda_lo_anterior() {
    let Some(api) = arrancar("guardada", true) else {
        return;
    };
    let texto = api
        .enviar("/api/conversations", &json!({}))
        .into_body()
        .read_to_string()
        .unwrap();
    let creada: Value = serde_json::from_str(&texto).unwrap();
    let ruta = format!("/api/conversations/{}", creada["id"]);

    // Primer mensaje, por trozos: el último evento trae lo guardado.
    let respuesta = api.enviar(
        &format!("{ruta}/messages"),
        &json!({"content": "Me llamo Ana. Responde solo «vale».", "model": "qwen", "temperature": 0,
                "system": "Responde en español, muy breve."}),
    );
    assert_eq!(respuesta.status().as_u16(), 200);
    let eventos: Vec<Value> = BufReader::new(respuesta.into_body().into_reader())
        .lines()
        .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
        .collect();
    let fin = eventos.last().unwrap();
    assert_eq!(fin["type"], "done");
    assert_eq!(fin["conversation"]["message_count"], 2);
    assert_eq!(
        fin["conversation"]["system"],
        "Responde en español, muy breve."
    );
    assert_eq!(fin["message"]["role"], "assistant");
    assert_eq!(fin["message"]["index"], 1);

    // Segundo, de una vez: el modelo recibe la conversación entera.
    let respuesta = api.enviar(
        &format!("{ruta}/messages"),
        &json!({"content": "¿Cómo me llamo? Responde solo el nombre.", "model": "qwen", "temperature": 0,
                "system": "Responde en español, muy breve.", "stream": false}),
    );
    assert_eq!(respuesta.status().as_u16(), 200);
    let fin: Value =
        serde_json::from_str(&respuesta.into_body().read_to_string().unwrap()).unwrap();
    let nombre = fin["message"]["content"].as_str().unwrap();
    eprintln!("respuesta: {nombre:?}");
    assert!(nombre.contains("Ana"), "recuerda el nombre: {nombre:?}");

    let abierta = api.leer(&ruta);
    assert_eq!(abierta["messages"].as_array().unwrap().len(), 4);
    assert_eq!(abierta["model"], "qwen");
}
