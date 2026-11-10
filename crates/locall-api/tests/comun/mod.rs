//! Ayudantes de las pruebas de la API: un Locall con su API en un puerto libre, apuntando a un Hugging Face falso.

#![allow(dead_code)] // cada archivo de pruebas usa solo parte

pub mod hf;

use std::path::PathBuf;
use std::sync::Arc;

use locall_config::Rutas;
use locall_core::{Nucleo, OpcionesNucleo};
use serde_json::Value;

pub struct Api {
    pub base: String,
    pub token: String,
    pub carpeta: PathBuf,
}

impl Drop for Api {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.carpeta);
    }
}

/// Arranca la API con un núcleo nuevo en una carpeta temporal, usando `hf` como Hugging Face.
pub fn arrancar(nombre: &str, hf: &hf::HfFalso) -> Api {
    let carpeta = std::env::temp_dir().join(format!("locall-apim-{nombre}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&carpeta);
    let rutas = Rutas::en(carpeta.clone());
    let mut opciones = OpcionesNucleo::nuevas(rutas.clone(), "no-hay-motor.exe");
    opciones.hugging_face = hf.base.clone();
    // El catálogo también al servidor falso: las pruebas no salen nunca a internet.
    opciones.catalogo = format!("{}/catalogo/catalogo.json", hf.base);
    let nucleo = Nucleo::abrir(opciones).unwrap();
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
    Api {
        base: format!("http://127.0.0.1:{puerto}"),
        token,
        carpeta,
    }
}

fn agente() -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build(),
    )
}

impl Api {
    /// Una petición con el token; devuelve la respuesta sin leer.
    pub fn pedir(
        &self,
        metodo: &str,
        ruta: &str,
        cuerpo: Option<&Value>,
    ) -> ureq::http::Response<ureq::Body> {
        let url = format!("{}{ruta}", self.base);
        let autorizacion = format!("Bearer {}", self.token);
        let agente = agente();
        match (metodo, cuerpo) {
            ("GET", _) => agente
                .get(&url)
                .header("Authorization", &autorizacion)
                .call(),
            ("DELETE", _) => agente
                .delete(&url)
                .header("Authorization", &autorizacion)
                .call(),
            ("PATCH", Some(c)) => agente
                .patch(&url)
                .header("Authorization", &autorizacion)
                .header("Content-Type", "application/json")
                .send(c.to_string()),
            (_, Some(c)) => agente
                .post(&url)
                .header("Authorization", &autorizacion)
                .header("Content-Type", "application/json")
                .send(c.to_string()),
            (_, None) => agente
                .post(&url)
                .header("Authorization", &autorizacion)
                .send_empty(),
        }
        .unwrap()
    }

    /// Como [`Api::pedir`], leyendo la respuesta como JSON.
    pub fn json(&self, metodo: &str, ruta: &str, cuerpo: Option<&Value>) -> (u16, Value) {
        let respuesta = self.pedir(metodo, ruta, cuerpo);
        let estado = respuesta.status().as_u16();
        let texto = respuesta.into_body().read_to_string().unwrap();
        (estado, serde_json::from_str(&texto).unwrap_or(Value::Null))
    }
}
