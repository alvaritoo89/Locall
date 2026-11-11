//! Malformed inputs against API requests (ADR 0026): any program on the computer (with the token) can send them. A
//! malformed body must give a 400 error, never crash Locall or make it allocate memory without limit. (axum already
//! limits the body to 2 MB.)

#[global_allocator]
static MEDIDOR: locall_mutador::MedidorDeMemoria = locall_mutador::MedidorDeMemoria;

fn bases(textos: &[&str]) -> Vec<Vec<u8>> {
    textos.iter().map(|t| t.as_bytes().to_vec()).collect()
}

#[test]
fn las_peticiones_de_chat_aguantan_entradas_basura() {
    let muestras = bases(&[
        r#"{"model":"qwen3-4b","messages":[{"role":"system","content":"Sé breve"},{"role":"user","content":"Hola ñ"}],"stream":true,"options":{"max_tokens":200,"temperature":0.7,"seed":1}}"#,
        r#"{"model":"x","messages":[{"role":"assistant","content":""}],"stream":false}"#,
    ]);
    locall_mutador::probar("api-chat", 0xA_C4A7, 20_000, &muestras, |entrada| {
        let _ = crate::chat::leer_peticion(entrada);
    });
}

#[test]
fn las_peticiones_de_conversaciones_aguantan_entradas_basura() {
    let muestras = bases(&[
        r#"{"content":"¿Qué tal?","model":"qwen3-4b","system":"Responde en español","temperature":1.1,"stream":false}"#,
        r#"{"content":"hola","model":"m"}"#,
    ]);
    locall_mutador::probar(
        "api-conversaciones",
        0xC0_4BE5,
        20_000,
        &muestras,
        |entrada| {
            let _ = crate::conversaciones::leer_peticion(entrada);
        },
    );
}

#[test]
fn las_peticiones_compatibles_con_openai_aguantan_entradas_basura() {
    let muestras = bases(&[
        r#"{"model":"qwen3-4b","messages":[{"role":"user","content":"Hola"}],"stream":true,"max_tokens":100,"temperature":0.2,"top_p":0.9,"seed":7,"stream_options":{"include_usage":true}}"#,
        r#"{"model":"m","messages":[{"role":"user","content":[{"type":"text","text":"partes"}]}]}"#,
    ]);
    locall_mutador::probar("api-openai", 0x0_9E4A1, 20_000, &muestras, |entrada| {
        let _ = crate::openai::leer_peticion(entrada);
    });
}
