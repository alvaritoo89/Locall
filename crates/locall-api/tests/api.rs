//! La API con un servidor de verdad (puerto libre en 127.0.0.1) y un núcleo en una carpeta temporal.
//!
//! Las peticiones se escriben a mano sobre TCP para poder poner cabeceras como las pondría un atacante
//! (`Host` falso, `Origin` de una web).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::Arc;

use locall_config::Rutas;
use locall_core::{Nucleo, OpcionesNucleo};
use serde_json::Value;

struct Prueba {
    puerto: u16,
    token: String,
    carpeta: PathBuf,
}

impl Drop for Prueba {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.carpeta);
    }
}

/// Arranca la API en un puerto libre, con un modelo importado («pequeno»).
fn arrancar(nombre: &str) -> Prueba {
    let carpeta = std::env::temp_dir().join(format!("locall-api-{nombre}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&carpeta);
    let rutas = Rutas::en(carpeta.clone());
    let nucleo = Nucleo::abrir(OpcionesNucleo::nuevas(rutas.clone(), "no-hay-motor.exe")).unwrap();

    // Un GGUF mínimo para tener algo en la lista: versión 3, 0 tensores, 1 metadato
    // (general.architecture = "llama").
    let mut gguf = b"GGUF".to_vec();
    gguf.extend(3u32.to_le_bytes());
    gguf.extend(0u64.to_le_bytes());
    gguf.extend(1u64.to_le_bytes());
    let clave = "general.architecture";
    gguf.extend((clave.len() as u64).to_le_bytes());
    gguf.extend(clave.as_bytes());
    gguf.extend(8u32.to_le_bytes()); // tipo: texto
    gguf.extend(5u64.to_le_bytes());
    gguf.extend(b"llama");
    let archivo = carpeta.join("Pequeno-Q4_0.gguf");
    std::fs::write(&archivo, gguf).unwrap();
    nucleo.importar(&archivo, Some("pequeno")).unwrap();

    let token = locall_api::token::leer_o_crear(&rutas.token()).unwrap();
    let escucha = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let puerto = escucha.local_addr().unwrap().port();
    escucha.set_nonblocking(true).unwrap();
    let rutas_api = locall_api::rutas(Arc::new(nucleo), token.clone(), puerto);
    std::thread::spawn(move || {
        let tiempo = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        tiempo.block_on(async {
            let escucha = tokio::net::TcpListener::from_std(escucha).unwrap();
            axum::serve(escucha, rutas_api).await.unwrap();
        });
    });
    Prueba {
        puerto,
        token,
        carpeta,
    }
}

impl Prueba {
    /// Una petición con estas cabeceras (si no se da `Host`, se pone el correcto). Devuelve (estado, JSON).
    fn pedir(&self, metodo: &str, ruta: &str, cabeceras: &[(&str, String)]) -> (u16, Value) {
        let mut texto = format!("{metodo} {ruta} HTTP/1.1\r\n");
        if !cabeceras
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("host"))
        {
            texto.push_str(&format!("Host: 127.0.0.1:{}\r\n", self.puerto));
        }
        for (nombre, valor) in cabeceras {
            texto.push_str(&format!("{nombre}: {valor}\r\n"));
        }
        texto.push_str("Connection: close\r\n\r\n");
        let mut conexion = TcpStream::connect(("127.0.0.1", self.puerto)).unwrap();
        conexion.write_all(texto.as_bytes()).unwrap();
        let mut respuesta = String::new();
        conexion.read_to_string(&mut respuesta).unwrap();
        let estado: u16 = respuesta[9..12].parse().unwrap();
        let cuerpo = respuesta.split_once("\r\n\r\n").map_or("", |(_, c)| c);
        (estado, serde_json::from_str(cuerpo).unwrap_or(Value::Null))
    }

    fn con_token(&self) -> (&'static str, String) {
        ("Authorization", format!("Bearer {}", self.token))
    }

    fn get(&self, ruta: &str) -> (u16, Value) {
        self.pedir("GET", ruta, &[self.con_token()])
    }
}

fn codigo(json: &Value) -> &str {
    json["error"]["code"].as_str().unwrap_or("")
}

#[test]
fn sin_token_no_se_puede_usar() {
    let api = arrancar("token");
    let (estado, json) = api.pedir("GET", "/api/models", &[]);
    assert_eq!((estado, codigo(&json)), (401, "token_required"));
    assert!(json["error"]["message"].as_str().unwrap().contains("token"));

    let malo = ("Authorization", format!("Bearer {}", "0".repeat(64)));
    let (estado, json) = api.pedir("GET", "/api/models", &[malo]);
    assert_eq!((estado, codigo(&json)), (401, "token_invalid"));

    // Ni siquiera se puede saber qué rutas existen sin token.
    let (estado, _) = api.pedir("GET", "/no/existe", &[]);
    assert_eq!(estado, 401);
}

#[test]
fn rechaza_un_host_que_no_es_localhost() {
    let api = arrancar("host");
    // DNS rebinding: la web «malvada.example» apunta su dominio a 127.0.0.1.
    for host in [
        "malvada.example".to_owned(),
        format!("malvada.example:{}", api.puerto),
        "localhost:1".to_owned(),
        "127.0.0.1.malvada.example".to_owned(),
    ] {
        let (estado, json) = api.pedir(
            "GET",
            "/api/version",
            &[("Host", host.clone()), api.con_token()],
        );
        assert_eq!((estado, codigo(&json)), (403, "host_not_allowed"), "{host}");
    }
    for host in [
        format!("localhost:{}", api.puerto),
        format!("127.0.0.1:{}", api.puerto),
        format!("[::1]:{}", api.puerto),
        "localhost".to_owned(),
    ] {
        let (estado, _) = api.pedir(
            "GET",
            "/api/version",
            &[("Host", host.clone()), api.con_token()],
        );
        assert_eq!(estado, 200, "{host}");
    }
}

#[test]
fn rechaza_las_peticiones_de_paginas_web() {
    let api = arrancar("origen");
    let web = ("Origin", "https://malvada.example".to_owned());
    // Aunque la web tuviera el token, no puede usar Locall.
    let (estado, json) = api.pedir("GET", "/api/models", &[web.clone(), api.con_token()]);
    assert_eq!((estado, codigo(&json)), (403, "origin_not_allowed"));
    // La pregunta previa que hace el navegador ("preflight") también se rechaza, y sin cabeceras CORS.
    let (estado, _) = api.pedir(
        "OPTIONS",
        "/api/models",
        &[web, ("Access-Control-Request-Method", "GET".into())],
    );
    assert_eq!(estado, 403);
}

#[test]
fn version_y_estado() {
    let api = arrancar("version");
    let (estado, json) = api.get("/api/version");
    assert_eq!(estado, 200);
    assert_eq!(json["name"], "locall");
    assert_eq!(json["engine"], locall_api::VERSION_MOTOR);

    let (estado, json) = api.get("/api/system");
    assert_eq!(estado, 200);
    assert!(json["threads"].as_u64().unwrap() >= 1);
    assert!(
        json["memory_total_bytes"].as_u64().unwrap()
            > json["memory_available_bytes"].as_u64().unwrap()
    );

    let (estado, json) = api.get("/api/status");
    assert_eq!(estado, 200);
    assert_eq!(json["state"], "idle");
    assert_eq!(json["model"], Value::Null);
    assert_eq!(
        json["model_memory_bytes"],
        Value::Null,
        "sin modelo cargado"
    );
    assert_eq!(json["unload_in_seconds"], Value::Null);
}

#[test]
fn lista_los_modelos_sin_rutas_del_disco() {
    let api = arrancar("modelos");
    let (estado, json) = api.get("/api/models");
    assert_eq!(estado, 200);
    let modelos = json["models"].as_array().unwrap();
    assert_eq!(modelos.len(), 1);
    let m = &modelos[0];
    assert_eq!(m["id"], "pequeno");
    assert_eq!(m["name"], "Pequeno-Q4_0");
    assert_eq!(m["quantization"], "Q4_0");
    assert_eq!(m["architecture"], "llama");
    assert_eq!(m["source"], "local");
    assert_eq!(m["files"][0]["sha256"].as_str().unwrap().len(), 64);
    // Un GGUF casi vacío cabe de sobra; el motor reconoce «llama».
    assert_eq!(m["fit"], "comfortable");
    assert_eq!(m["compatible"], true);
    assert!(m["memory_estimate_bytes"].as_u64().is_some());
    let texto = json.to_string();
    assert!(
        !texto.contains(":\\\\") && !texto.contains("locall-api-"),
        "sin rutas: {texto}"
    );

    let (estado, json) = api.get("/api/models/pequeno");
    assert_eq!((estado, json["id"].as_str()), (200, Some("pequeno")));

    let (estado, json) = api.get("/api/models/no-esta");
    assert_eq!((estado, codigo(&json)), (404, "model_not_found"));
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("«no-esta»")
    );

    let (estado, json) = api.get("/api/no-existe");
    assert_eq!((estado, codigo(&json)), (404, "not_found"));
}
