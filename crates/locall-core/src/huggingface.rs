//! Consultas a la API pública de Hugging Face (sin cuenta ni token en la v1).
//!
//! - `GET /api/models/{repo}/revision/{revision}` → el **commit exacto** (`sha`), la licencia (etiqueta
//!   `license:…`) y si el repositorio está protegido (`gated`).
//! - `GET /api/models/{repo}/tree/{commit}?recursive=true` → los archivos, con su tamaño y, en los grandes (LFS), su
//!   **sha256** (`lfs.oid`): la misma huella que verifica el descargador.
//! - `GET /{repo}/resolve/{commit}/{ruta}` → el archivo (Hugging Face redirige a su red de distribución).
//!
//! La dirección base se puede cambiar para las pruebas (un servidor falso local).

use std::time::Duration;

use locall_gguf::Cabecera;
use locall_store::Huella;
use serde_json::Value;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

use crate::ErrorNucleo;

/// La dirección de Hugging Face.
pub const HUGGING_FACE: &str = "https://huggingface.co";

/// Datos de una revisión.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfoRevision {
    /// Commit exacto (40 caracteres hexadecimales).
    pub commit: String,
    /// Licencia declarada en el repositorio (`mit`, `apache-2.0`…), si la hay.
    pub licencia: Option<String>,
    /// Hay que aceptar condiciones con una cuenta para descargarlo.
    pub protegido: bool,
}

/// Un archivo del repositorio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivoHf {
    /// Ruta dentro del repositorio (puede tener carpetas: `Q4_K_M/modelo-00001-of-00002.gguf`).
    pub ruta: String,
    pub bytes: u64,
    /// sha256, solo en los archivos grandes (LFS). Sin ella no se puede verificar la descarga.
    pub huella: Option<Huella>,
}

impl ArchivoHf {
    /// El nombre del archivo, sin carpetas.
    pub fn nombre(&self) -> &str {
        self.ruta.rsplit('/').next().unwrap_or(&self.ruta)
    }
}

/// Cliente de la API de Hugging Face.
pub(crate) struct HuggingFace {
    agente: ureq::Agent,
    base: String,
}

impl HuggingFace {
    pub(crate) fn nuevo(base: &str) -> HuggingFace {
        let tls = TlsConfig::builder()
            .provider(TlsProvider::NativeTls)
            .root_certs(RootCerts::PlatformVerifier)
            .build();
        let config = ureq::Agent::config_builder()
            .tls_config(tls)
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_per_read(Some(Duration::from_secs(60)))
            .user_agent(format!("locall/{}", env!("CARGO_PKG_VERSION")))
            .build();
        HuggingFace {
            agente: ureq::Agent::new_with_config(config),
            base: base.trim_end_matches('/').to_owned(),
        }
    }

    /// Resuelve una revisión (rama, etiqueta o commit) a su commit exacto.
    pub(crate) fn revision(&self, repo: &str, revision: &str) -> Result<InfoRevision, ErrorNucleo> {
        let url = format!(
            "{}/api/models/{repo}/revision/{}",
            self.base,
            revision.replace('/', "%2F")
        );
        let valor = self.json(&url, repo)?;
        let commit = valor["sha"]
            .as_str()
            .filter(|s| s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit()))
            .ok_or_else(|| ErrorNucleo::RespuestaHf("la revisión no trae un commit válido".into()))?
            .to_ascii_lowercase();
        let licencia = valor["tags"].as_array().and_then(|etiquetas| {
            etiquetas
                .iter()
                .filter_map(Value::as_str)
                .find_map(|e| e.strip_prefix("license:"))
                .map(str::to_owned)
        });
        // `gated` es `false` o el tipo de protección ("auto", "manual").
        let protegido = !matches!(valor["gated"], Value::Bool(false) | Value::Null);
        Ok(InfoRevision {
            commit,
            licencia,
            protegido,
        })
    }

    /// Todos los archivos del repositorio en ese commit.
    pub(crate) fn archivos(&self, repo: &str, commit: &str) -> Result<Vec<ArchivoHf>, ErrorNucleo> {
        let url = format!(
            "{}/api/models/{repo}/tree/{commit}?recursive=true",
            self.base
        );
        let valor = self.json(&url, repo)?;
        let lista = valor.as_array().ok_or_else(|| {
            ErrorNucleo::RespuestaHf("la lista de archivos no es una lista".into())
        })?;
        let mut archivos = Vec::new();
        for entrada in lista {
            if entrada["type"].as_str() != Some("file") {
                continue;
            }
            let (Some(ruta), Some(bytes)) = (entrada["path"].as_str(), entrada["size"].as_u64())
            else {
                continue;
            };
            let huella = entrada["lfs"]["oid"]
                .as_str()
                .and_then(|h| Huella::desde_hex(h).ok());
            archivos.push(ArchivoHf {
                ruta: ruta.to_owned(),
                bytes,
                huella,
            });
        }
        Ok(archivos)
    }

    /// Dirección de descarga de un archivo, fijada al commit.
    pub(crate) fn url_archivo(&self, repo: &str, commit: &str, ruta: &str) -> String {
        format!(
            "{}/{repo}/resolve/{commit}/{}",
            self.base,
            ruta.replace(' ', "%20")
        )
    }

    /// Lee **solo la cabecera** de un GGUF remoto: se piden los datos por la red y se deja de leer en cuanto termina
    /// la cabecera (unos MB, aunque el modelo ocupe varios GB). Así se comprueba la compatibilidad ANTES de
    /// descargarlo entero.
    pub(crate) fn leer_cabecera(&self, url: &str) -> Result<Cabecera, ErrorNucleo> {
        let respuesta = self
            .agente
            .get(url)
            .call()
            .map_err(|e| ErrorNucleo::Red(e.to_string()))?;
        let estado = respuesta.status().as_u16();
        if estado != 200 {
            return Err(ErrorNucleo::HuggingFace { estado });
        }
        let lector = respuesta.into_body().into_reader();
        // Al salir de aquí se suelta el lector y se cierra la conexión: el resto del archivo no se descarga.
        locall_gguf::leer(lector).map_err(ErrorNucleo::Cabecera)
    }

    fn json(&self, url: &str, repo: &str) -> Result<Value, ErrorNucleo> {
        let respuesta = self
            .agente
            .get(url)
            .call()
            .map_err(|e| ErrorNucleo::Red(e.to_string()))?;
        match respuesta.status().as_u16() {
            200 => {}
            // Hugging Face responde 401 (no 404) a los repositorios que no existen o son privados.
            401 | 404 => return Err(ErrorNucleo::RepoNoEncontrado(repo.to_owned())),
            estado => return Err(ErrorNucleo::HuggingFace { estado }),
        }
        let texto = respuesta
            .into_body()
            .with_config()
            .limit(32 * 1024 * 1024) // una lista de archivos enorme no debe agotar la memoria
            .read_to_string()
            .map_err(|e| ErrorNucleo::Red(e.to_string()))?;
        serde_json::from_str(&texto).map_err(|e| ErrorNucleo::RespuestaHf(e.to_string()))
    }
}
