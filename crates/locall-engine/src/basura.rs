//! Malformed inputs against what comes from the engine (ADR 0026): each chunk of the answer, HTTP errors and their
//! log. We launched the engine ourselves, but if a malicious model made it write strange things, Locall must not
//! crash.

use crate::chat::{error_de_respuesta, interpretar};
use crate::errores::causa_en_registro;

#[global_allocator]
static MEDIDOR: locall_mutador::MedidorDeMemoria = locall_mutador::MedidorDeMemoria;

fn bases(textos: &[&str]) -> Vec<Vec<u8>> {
    textos.iter().map(|t| t.as_bytes().to_vec()).collect()
}

#[test]
fn los_trozos_de_la_respuesta_aguantan_entradas_basura() {
    let muestras = bases(&[
        r#"{"choices":[{"finish_reason":null,"index":0,"delta":{"role":"assistant","content":null}}],"prompt_progress":{"total":32,"cache":0,"processed":32,"time_ms":178}}"#,
        r#"{"choices":[{"finish_reason":null,"index":0,"delta":{"content":"Hola ñ 🙂"}}]}"#,
        r#"{"choices":[{"finish_reason":null,"delta":{"reasoning_content":"Veamos"}}]}"#,
        r#"{"choices":[{"finish_reason":"length","index":0,"delta":{}}],"timings":{"cache_n":0,"prompt_n":32,"prompt_ms":180.8,"prompt_per_second":176.9,"predicted_n":8,"predicted_ms":201.0,"predicted_per_second":34.8}}"#,
        r#"{"error":{"code":500,"message":"fallo","type":"server_error"}}"#,
    ]);
    locall_mutador::probar("motor-trozos", 0x7_2020, 20_000, &muestras, |entrada| {
        let _ = interpretar(&String::from_utf8_lossy(entrada));
    });
}

#[test]
fn los_errores_del_motor_aguantan_entradas_basura() {
    let muestras = bases(&[
        r#"{"error":{"code":400,"message":"request (6030 tokens) exceeds the available context size (2048 tokens), try increasing it","type":"exceed_context_size_error","n_prompt_tokens":6030,"n_ctx":2048}}"#,
        r#"{"error":{"code":503,"message":"Loading model","type":"unavailable_error"}}"#,
        "<html>502</html>",
    ]);
    locall_mutador::probar("motor-errores", 0xE_4404, 20_000, &muestras, |entrada| {
        let texto = String::from_utf8_lossy(entrada);
        for estado in [400, 500, 503] {
            let _ = error_de_respuesta(estado, &texto).to_string();
        }
    });
}

#[test]
fn el_registro_del_motor_aguanta_entradas_basura() {
    let muestras = bases(&[
        "0.00.020.056 E srv         start: couldn't bind HTTP server socket, hostname: 127.0.0.1, port: 18993\n0.00.020.177 E srv  server_run_i: exiting due to HTTP server error",
        "0.00.037.414 E llama_model_load: error loading model: unknown model architecture: 'inventada'",
        "0.00.058.140 E llama_model_load: error loading model: tensor 'token_embd.weight' data is not within the file bounds, model is corrupted or incomplete",
        "0.00.886.245 E llama_init_from_model: failed to initialize the context: failed to allocate buffer for kv cache",
        "request (6030 tokens) exceeds the available context size (2048 tokens)",
    ]);
    locall_mutador::probar("motor-registro", 0x4E6_1570, 20_000, &muestras, |entrada| {
        let _ = causa_en_registro(&String::from_utf8_lossy(entrada));
    });
}
