//! Queries to Hugging Face's public API (no account or token in v1).
//!
//! - `GET /api/models/{repo}/revision/{revision}` → the **exact commit** (`sha`), the license (`license:…` tag) and
//!   whether the repository is gated (`gated`).
//! - `GET /api/models/{repo}/tree/{commit}?recursive=true` → the files, with their size and, for the large ones
//!   (LFS), their **sha256** (`lfs.oid`): the same hash the downloader verifies.
//! - `GET /{repo}/resolve/{commit}/{path}` → the file (Hugging Face redirects to its distribution network).
//! - `GET /api/models?search=…&filter=gguf&sort=downloads&direction=-1&limit=…` → **search** repositories with GGUF
//!   by name, most downloaded first. Only the searched text is sent.
//!
//! The base address can be changed for tests (a local fake server).

use std::time::Duration;

use locall_gguf::Cabecera;
use locall_store::Huella;
use serde_json::Value;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

use crate::ErrorNucleo;

/// Hugging Face's address.
pub const HUGGING_FACE: &str = "https://huggingface.co";

/// Data of a revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfoRevision {
    /// Exact commit (40 hexadecimal characters).
    pub commit: String,
    /// License declared in the repository (`mit`, `apache-2.0`…), if any.
    pub licencia: Option<String>,
    /// Conditions must be accepted with an account to download it.
    pub protegido: bool,
}

/// A file of the repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivoHf {
    /// Path inside the repository (may have folders: `Q4_K_M/model-00001-of-00002.gguf`).
    pub ruta: String,
    pub bytes: u64,
    /// sha256, only for large files (LFS). Without it the download cannot be verified.
    pub huella: Option<Huella>,
}

impl ArchivoHf {
    /// The file name, without folders.
    pub fn nombre(&self) -> &str {
        self.ruta.rsplit('/').next().unwrap_or(&self.ruta)
    }
}

/// A repository found by searching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoEncontrado {
    /// `owner/name`.
    pub repo: String,
    pub descargas: u64,
    pub me_gusta: u64,
    /// Declared license (`apache-2.0`…), if any.
    pub licencia: Option<String>,
    /// What it is for according to Hugging Face (`text-generation`, `image-text-to-text`…), if it says.
    pub tarea: Option<String>,
}

/// The license from a list of Hugging Face tags (`license:mit` → `mit`).
fn licencia_de(etiquetas: &Value) -> Option<String> {
    etiquetas.as_array().and_then(|etiquetas| {
        etiquetas
            .iter()
            .filter_map(Value::as_str)
            .find_map(|e| e.strip_prefix("license:"))
            .map(str::to_owned)
    })
}

/// Encodes a text to put it in an address (`qwen 3` → `qwen%203`): everything that is not an ASCII letter or digit,
/// `-`, `_`, `.` or `~` is written as `%XX` (each byte of its UTF-8).
pub(crate) fn codificar(texto: &str) -> String {
    let mut resultado = String::with_capacity(texto.len());
    for byte in texto.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            resultado.push(char::from(byte));
        } else {
            resultado.push_str(&format!("%{byte:02X}"));
        }
    }
    resultado
}

/// An HTTPS client with Windows' TLS (ADR 0010), without turning HTTP errors into errors (they are checked by hand).
pub(crate) fn agente_https() -> ureq::Agent {
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
    ureq::Agent::new_with_config(config)
}

/// Hugging Face API client.
pub(crate) struct HuggingFace {
    agente: ureq::Agent,
    base: String,
}

impl HuggingFace {
    pub(crate) fn nuevo(base: &str) -> HuggingFace {
        HuggingFace {
            agente: agente_https(),
            base: base.trim_end_matches('/').to_owned(),
        }
    }

    /// Resolves a revision (branch, tag or commit) to its exact commit.
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
        let licencia = licencia_de(&valor["tags"]);
        // `gated` is `false` or the kind of gating ("auto", "manual").
        let protegido = !matches!(valor["gated"], Value::Bool(false) | Value::Null);
        Ok(InfoRevision {
            commit,
            licencia,
            protegido,
        })
    }

    /// All the files of the repository at that commit.
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

    /// Searches repositories with GGUF by name, most downloaded first.
    pub(crate) fn buscar(
        &self,
        texto: &str,
        limite: u32,
    ) -> Result<Vec<RepoEncontrado>, ErrorNucleo> {
        let url = format!(
            "{}/api/models?search={}&filter=gguf&sort=downloads&direction=-1&limit={limite}",
            self.base,
            codificar(texto)
        );
        let valor = self.json(&url, texto)?;
        let lista = valor
            .as_array()
            .ok_or_else(|| ErrorNucleo::RespuestaHf("la búsqueda no devuelve una lista".into()))?;
        Ok(lista
            .iter()
            .filter(|r| r["private"].as_bool() != Some(true))
            .filter_map(|r| {
                Some(RepoEncontrado {
                    repo: r["id"].as_str()?.to_owned(),
                    descargas: r["downloads"].as_u64().unwrap_or(0),
                    me_gusta: r["likes"].as_u64().unwrap_or(0),
                    licencia: licencia_de(&r["tags"]),
                    tarea: r["pipeline_tag"].as_str().map(str::to_owned),
                })
            })
            .collect())
    }

    /// Download address of a file, pinned to the commit.
    pub(crate) fn url_archivo(&self, repo: &str, commit: &str, ruta: &str) -> String {
        format!(
            "{}/{repo}/resolve/{commit}/{}",
            self.base,
            ruta.replace(' ', "%20")
        )
    }

    /// Reads **only the header** of a remote GGUF: the data is requested over the network and reading stops as soon as
    /// the header ends (a few MB, even if the model takes several GB). This way compatibility is checked BEFORE
    /// downloading it whole.
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
        // On leaving here the reader is dropped and the connection closed: the rest of the file is not downloaded.
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
            // Hugging Face answers 401 (not 404) for repositories that do not exist or are private.
            401 | 404 => return Err(ErrorNucleo::RepoNoEncontrado(repo.to_owned())),
            estado => return Err(ErrorNucleo::HuggingFace { estado }),
        }
        let texto = respuesta
            .into_body()
            .with_config()
            .limit(32 * 1024 * 1024) // a huge file list must not exhaust memory
            .read_to_string()
            .map_err(|e| ErrorNucleo::Red(e.to_string()))?;
        serde_json::from_str(&texto).map_err(|e| ErrorNucleo::RespuestaHf(e.to_string()))
    }
}

#[cfg(test)]
mod pruebas {
    use super::codificar;

    #[test]
    fn codifica_lo_que_no_es_seguro_en_una_direccion() {
        assert_eq!(codificar("Qwen3-4B_v1.0~x"), "Qwen3-4B_v1.0~x");
        assert_eq!(codificar("qwen 3"), "qwen%203");
        assert_eq!(codificar("a&b=c?d/e#f"), "a%26b%3Dc%3Fd%2Fe%23f");
        assert_eq!(codificar("español"), "espa%C3%B1ol");
    }
}
