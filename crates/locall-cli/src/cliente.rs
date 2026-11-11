//! Local API client: the CLI does not touch the core directly, it talks to it over HTTP like any other program
//! (rule 9, ADR 0017).

use std::fmt;
use std::path::Path;
use std::time::Duration;

use serde_json::Value;

/// What can go wrong in the CLI.
#[derive(Debug)]
pub enum ErrorCli {
    /// The command is malformed.
    Uso(String),
    /// Locall is not open (for the commands that do not start it).
    NoAbierto,
    /// The core could not be started.
    Arranque(String),
    /// The API answered with an error (its message is already in Spanish).
    Api {
        estado: u16,
        codigo: String,
        mensaje: String,
    },
    /// Connection or read failure.
    Red(String),
    /// Configuration or token.
    Config(String),
}

impl fmt::Display for ErrorCli {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorCli::Uso(m) => write!(f, "{m}\nEscribe «locall help» para ver las órdenes."),
            ErrorCli::NoAbierto => write!(f, "Locall no está abierto"),
            ErrorCli::Arranque(m) => write!(f, "no se pudo arrancar Locall: {m}"),
            ErrorCli::Api { mensaje, .. } => write!(f, "{mensaje}"),
            ErrorCli::Red(m) => write!(f, "no se pudo hablar con Locall: {m}"),
            ErrorCli::Config(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for ErrorCli {}

/// An API client, already with its address and token.
#[derive(Clone)]
pub struct Cliente {
    base: String,
    token: String,
    agente: ureq::Agent,
    /// To check whether anyone is listening, with a short timeout: on Windows, connecting to a closed `localhost` port
    /// takes ~2 s (the system retries), but if the server exists it accepts instantly.
    sonda: ureq::Agent,
}

impl Cliente {
    pub fn nuevo(base: impl Into<String>, token: impl Into<String>) -> Cliente {
        let agente = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .http_status_as_error(false)
                .timeout_connect(Some(Duration::from_secs(2)))
                .build(),
        );
        let sonda = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .http_status_as_error(false)
                .timeout_connect(Some(Duration::from_millis(300)))
                .timeout_global(Some(Duration::from_secs(5)))
                .build(),
        );
        Cliente {
            base: base.into(),
            token: token.into(),
            agente,
            sonda,
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// `GET` that returns JSON.
    pub fn get(&self, ruta: &str) -> Result<Value, ErrorCli> {
        let respuesta = self
            .agente
            .get(&format!("{}{ruta}", self.base))
            .header("Authorization", &self.autorizacion())
            .call()
            .map_err(|e| ErrorCli::Red(e.to_string()))?;
        leer_json(respuesta)
    }

    /// `POST` with a JSON body that returns JSON.
    pub fn post(&self, ruta: &str, cuerpo: &Value) -> Result<Value, ErrorCli> {
        leer_json(self.post_sin_leer(ruta, cuerpo)?)
    }

    /// `POST` that returns the response unread (for chunked responses). If it is an error, it returns it already
    /// converted.
    pub fn post_sin_leer(
        &self,
        ruta: &str,
        cuerpo: &Value,
    ) -> Result<ureq::http::Response<ureq::Body>, ErrorCli> {
        let respuesta = self
            .agente
            .post(&format!("{}{ruta}", self.base))
            .header("Authorization", &self.autorizacion())
            .header("Content-Type", "application/json")
            .send(cuerpo.to_string())
            .map_err(|e| ErrorCli::Red(e.to_string()))?;
        if respuesta.status().is_success() {
            Ok(respuesta)
        } else {
            Err(leer_json(respuesta).expect_err("una respuesta de error es un error"))
        }
    }

    /// `PATCH` with a JSON body that returns JSON.
    pub fn patch(&self, ruta: &str, cuerpo: &Value) -> Result<Value, ErrorCli> {
        let respuesta = self
            .agente
            .patch(&format!("{}{ruta}", self.base))
            .header("Authorization", &self.autorizacion())
            .header("Content-Type", "application/json")
            .send(cuerpo.to_string())
            .map_err(|e| ErrorCli::Red(e.to_string()))?;
        leer_json(respuesta)
    }

    /// `DELETE` that returns JSON.
    pub fn delete(&self, ruta: &str) -> Result<Value, ErrorCli> {
        let respuesta = self
            .agente
            .delete(&format!("{}{ruta}", self.base))
            .header("Authorization", &self.autorizacion())
            .call()
            .map_err(|e| ErrorCli::Red(e.to_string()))?;
        leer_json(respuesta)
    }

    /// Does the API answer with our token? (If another program is on the port, or a Locall with another token, no.)
    pub fn responde(&self) -> bool {
        self.sonda
            .get(&format!("{}/api/version", self.base))
            .header("Authorization", &self.autorizacion())
            .call()
            .is_ok_and(|r| r.status().is_success())
    }

    fn autorizacion(&self) -> String {
        format!("Bearer {}", self.token)
    }
}

fn leer_json(respuesta: ureq::http::Response<ureq::Body>) -> Result<Value, ErrorCli> {
    let estado = respuesta.status().as_u16();
    let texto = respuesta
        .into_body()
        .read_to_string()
        .map_err(|e| ErrorCli::Red(e.to_string()))?;
    let json: Value = serde_json::from_str(&texto).unwrap_or(Value::Null);
    if (200..300).contains(&estado) {
        return Ok(json);
    }
    Err(ErrorCli::Api {
        estado,
        codigo: json["error"]["code"].as_str().unwrap_or("").to_owned(),
        mensaje: json["error"]["message"].as_str().map_or_else(
            || format!("la API respondió con el error {estado}"),
            str::to_owned,
        ),
    })
}

/// The API token, if it exists and is valid.
pub fn leer_token(ruta: &Path) -> Option<String> {
    let texto = std::fs::read_to_string(ruta).ok()?;
    let token = texto.trim();
    (token.len() == 64 && token.chars().all(|c| c.is_ascii_hexdigit())).then(|| token.to_owned())
}
