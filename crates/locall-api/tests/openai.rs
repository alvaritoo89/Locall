//! OpenAI compatibility (`/v1/...`) and idle shutdown.

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
    nucleo: Arc<Nucleo>,
}

impl Drop for Prueba {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.carpeta);
    }
}

fn hay_motor() -> bool {
    let hay = PathBuf::from(MOTOR).is_file() && PathBuf::from(MODELO).is_file();
    if !hay {
        eprintln!("(prueba saltada: faltan el motor o el modelo)");
    }
    hay
}

/// The API with the Phase 0 model imported as «qwen» (`real`) or with a minimal GGUF as «mini».
fn arrancar(nombre: &str, real: bool) -> Prueba {
    let carpeta = std::env::temp_dir().join(format!("locall-oai-{nombre}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&carpeta);
    std::fs::create_dir_all(&carpeta).unwrap();
    let rutas = Rutas::en(carpeta.clone());
    let nucleo = Arc::new(Nucleo::abrir(OpcionesNucleo::nuevas(rutas.clone(), MOTOR)).unwrap());
    if real {
        nucleo
            .importar(&PathBuf::from(MODELO), Some("qwen"))
            .unwrap();
    } else {
        let mut gguf = b"GGUF".to_vec();
        gguf.extend(3u32.to_le_bytes());
        gguf.extend(0u64.to_le_bytes());
        gguf.extend(1u64.to_le_bytes());
        let clave = "general.architecture";
        gguf.extend((clave.len() as u64).to_le_bytes());
        gguf.extend(clave.as_bytes());
        gguf.extend(8u32.to_le_bytes());
        gguf.extend(5u64.to_le_bytes());
        gguf.extend(b"llama");
        let archivo = carpeta.join("mini.gguf");
        std::fs::write(&archivo, gguf).unwrap();
        nucleo.importar(&archivo, Some("mini")).unwrap();
    }
    let token = locall_api::token::leer_o_crear(&rutas.token()).unwrap();
    let escucha = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let puerto = escucha.local_addr().unwrap().port();
    escucha.set_nonblocking(true).unwrap();
    let rutas_api = locall_api::rutas(Arc::clone(&nucleo), token.clone(), puerto);
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
    Prueba {
        base: format!("http://127.0.0.1:{puerto}"),
        token,
        carpeta,
        nucleo,
    }
}

impl Prueba {
    fn post(&self, ruta: &str, cuerpo: &Value) -> ureq::http::Response<ureq::Body> {
        ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .http_status_as_error(false)
                .build(),
        )
        .post(&format!("{}{ruta}", self.base))
        .header("Authorization", &format!("Bearer {}", self.token))
        .header("Content-Type", "application/json")
        .send(cuerpo.to_string())
        .unwrap()
    }

    fn post_json(&self, ruta: &str, cuerpo: &Value) -> (u16, Value) {
        let r = self.post(ruta, cuerpo);
        let estado = r.status().as_u16();
        (
            estado,
            serde_json::from_str(&r.into_body().read_to_string().unwrap()).unwrap_or(Value::Null),
        )
    }

    fn get_json(&self, ruta: &str) -> Value {
        let texto = ureq::get(&format!("{}{ruta}", self.base))
            .header("Authorization", &format!("Bearer {}", self.token))
            .call()
            .unwrap()
            .into_body()
            .read_to_string()
            .unwrap();
        serde_json::from_str(&texto).unwrap()
    }
}

#[test]
fn lista_los_modelos_al_estilo_openai() {
    let api = arrancar("modelos", false);
    let json = api.get_json("/v1/models");
    assert_eq!(json["object"], "list");
    assert_eq!(json["data"][0]["id"], "mini");
    assert_eq!(json["data"][0]["object"], "model");
    assert!(json["data"][0]["created"].as_u64().unwrap() > 1_700_000_000);
}

#[test]
fn rechaza_con_claridad_lo_que_aun_no_se_admite() {
    let api = arrancar("no-admite", false);
    let base = |extra: Value| {
        let mut c = json!({"model": "mini", "messages": [{"role": "user", "content": "hola"}]});
        for (k, v) in extra.as_object().unwrap() {
            c[k] = v.clone();
        }
        c
    };
    let casos = [
        (base(json!({"tools": [{"type": "function"}]})), "«tools»"),
        (
            base(json!({"response_format": {"type": "json_object"}})),
            "«response_format»",
        ),
        (base(json!({"n": 2})), "una respuesta"),
        (
            json!({"model": "mini", "messages": [{"role": "user", "content": [
                {"type": "image_url", "image_url": {"url": "x"}}]}]}),
            "«image_url»",
        ),
        (
            json!({"model": "mini", "messages": [{"role": "tool", "content": "x"}]}),
            "«tool»",
        ),
    ];
    for (cuerpo, fragmento) in casos {
        let (estado, json) = api.post_json("/v1/chat/completions", &cuerpo);
        assert_eq!(estado, 400, "{cuerpo}");
        let mensaje = json["error"]["message"].as_str().unwrap();
        assert!(mensaje.contains(fragmento), "{cuerpo}: {mensaje}");
    }
    let (estado, json) = api.post_json(
        "/v1/chat/completions",
        &json!({"model": "otro", "messages": [{"role": "user", "content": "hola"}]}),
    );
    assert_eq!(
        (estado, json["error"]["code"].as_str()),
        (404, Some("model_not_found"))
    );
}

#[test]
fn responde_entero_como_openai() {
    if !hay_motor() {
        return;
    }
    let api = arrancar("entero", true);
    let (estado, json) = api.post_json(
        "/v1/chat/completions",
        &json!({
            "model": "qwen",
            // Content as a list of parts (OpenAI's newer format) and the "developer" role.
            "messages": [
                {"role": "developer", "content": "Responde en una palabra."},
                {"role": "user", "content": [{"type": "text", "text": "¿Capital de Francia?"}]}
            ],
            "max_tokens": 10, "temperature": 0, "seed": 1
        }),
    );
    assert_eq!(estado, 200, "{json}");
    assert_eq!(json["object"], "chat.completion");
    assert!(json["id"].as_str().unwrap().starts_with("chatcmpl-"));
    assert_eq!(json["model"], "qwen");
    let eleccion = &json["choices"][0];
    assert_eq!(eleccion["message"]["role"], "assistant");
    let texto = eleccion["message"]["content"].as_str().unwrap();
    eprintln!("respuesta: {texto:?}");
    assert!(texto.contains("Par"), "{texto}");
    assert!(matches!(
        eleccion["finish_reason"].as_str(),
        Some("stop" | "length")
    ));
    let uso = &json["usage"];
    assert_eq!(
        uso["total_tokens"].as_u64().unwrap(),
        uso["prompt_tokens"].as_u64().unwrap() + uso["completion_tokens"].as_u64().unwrap()
    );
}

#[test]
fn responde_por_sse_como_openai() {
    if !hay_motor() {
        return;
    }
    let api = arrancar("sse", true);
    let respuesta = api.post(
        "/v1/chat/completions",
        &json!({
            "model": "qwen", "stream": true, "stream_options": {"include_usage": true},
            "messages": [{"role": "user", "content": "¿Capital de Francia? Una palabra."}],
            "max_tokens": 10, "temperature": 0, "seed": 1
        }),
    );
    assert_eq!(respuesta.status().as_u16(), 200);
    assert!(
        respuesta.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/event-stream")
    );

    let lineas: Vec<String> = BufReader::new(respuesta.into_body().into_reader())
        .lines()
        .map(Result::unwrap)
        .filter(|l| !l.is_empty())
        .collect();
    assert!(lineas.iter().all(|l| l.starts_with("data: ")), "{lineas:?}");
    assert_eq!(lineas.last().unwrap(), "data: [DONE]");
    let trozos: Vec<Value> = lineas[..lineas.len() - 1]
        .iter()
        .map(|l| serde_json::from_str(&l["data: ".len()..]).unwrap())
        .collect();
    assert!(
        trozos
            .iter()
            .all(|t| t["object"] == "chat.completion.chunk")
    );
    assert_eq!(trozos[0]["choices"][0]["delta"]["role"], "assistant");
    let texto: String = trozos
        .iter()
        .filter_map(|t| t["choices"][0]["delta"]["content"].as_str())
        .collect();
    eprintln!("respuesta: {texto:?}");
    assert!(texto.contains("Par"), "{texto}");
    // The second to last carries the finish reason; the last one, the usage (with an empty choices list).
    let n = trozos.len();
    assert!(trozos[n - 2]["choices"][0]["finish_reason"].is_string());
    assert_eq!(trozos[n - 1]["choices"], json!([]));
    assert!(
        trozos[n - 1]["usage"]["completion_tokens"]
            .as_u64()
            .unwrap()
            > 0
    );
}

#[test]
fn se_cierra_solo_tras_un_rato_sin_uso() {
    let api = arrancar("inactividad", false);
    let tiempo = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let actividad = locall_api::Actividad::nueva();

    // With continuous use (a "request" every 100 ms) it does not shut down in 800 ms even though the limit is 300 ms.
    let inicio = Instant::now();
    let nucleo = Arc::clone(&api.nucleo);
    let tocador = actividad.clone();
    let cerrado = tiempo.block_on(async {
        let espera = locall_api::esperar_inactividad(
            nucleo,
            actividad.clone(),
            Duration::from_millis(300),
            Duration::from_millis(50),
        );
        let uso = async {
            while inicio.elapsed() < Duration::from_millis(800) {
                tocador.tocar();
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        };
        // Whichever ends first: the use (800 ms) or the idle wait.
        let espera = std::pin::pin!(espera);
        let uso = std::pin::pin!(uso);
        matches!(
            futures_util::future::select(espera, uso).await,
            futures_util::future::Either::Left(_)
        )
    });
    assert!(!cerrado, "se cerró aunque se estaba usando");

    // Without use, it finishes once the limit has passed (counted from the last use).
    actividad.tocar();
    let inicio = Instant::now();
    tiempo.block_on(locall_api::esperar_inactividad(
        Arc::clone(&api.nucleo),
        actividad,
        Duration::from_millis(300),
        Duration::from_millis(50),
    ));
    let tardo = inicio.elapsed();
    assert!(
        tardo >= Duration::from_millis(290) && tardo < Duration::from_secs(2),
        "{tardo:?}"
    );
}
