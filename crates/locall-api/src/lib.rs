//! # locall-api
//!
//! La API local de Locall: un servidor HTTP (axum) que solo escucha en `127.0.0.1` y traduce entre JSON y el
//! [`Nucleo`] (ADR 0015). No contiene lógica propia: todo lo hace el núcleo (regla 9).
//!
//! **Seguridad**, en este orden, antes de cualquier ruta (incluidas las que no existen):
//!
//! 1. **`Host`** debe ser `127.0.0.1`, `localhost` o `[::1]` (con nuestro puerto). Bloquea el "DNS rebinding": una
//!    web maliciosa que hace que su dominio apunte a 127.0.0.1 para hablar con programas de tu equipo.
//! 2. **`Origin`**: las peticiones que hace un navegador desde una web llevan esta cabecera; se rechazan todas (la
//!    app de escritorio tendrá su origen permitido cuando exista). Así ninguna web que visites puede usar Locall.
//! 3. **Token** (`Authorization: Bearer …`), comparado en tiempo constante.
//!
//! Los errores tienen siempre la forma `{"error": {"code": "...", "message": "..."}}`: `code` en inglés y fijo, para
//! los programas; `message` en español, para las personas.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use locall_core::{ErrorNucleo, Nucleo};
use locall_engine::{Causa, ErrorMotor};
use locall_registry::Modelo;
use locall_scheduler::ErrorCarga;
use locall_scheduler::Estado;
use serde_json::{Value, json};

mod chat;
pub mod token;

/// Versión del motor con la que se ha probado esta versión de Locall.
pub const VERSION_MOTOR: &str = "llama.cpp b12189";

// ─── Construcción ─────────────────────────────────────────────────────────────

/// Lo que comparten todas las peticiones.
#[derive(Clone)]
struct Compartido {
    nucleo: Arc<Nucleo>,
    token: Arc<str>,
    puerto: u16,
}

/// Construye las rutas de la API con su seguridad. `puerto` es en el que se escucha (para validar `Host`).
pub fn rutas(nucleo: Arc<Nucleo>, token: String, puerto: u16) -> Router {
    let compartido = Compartido {
        nucleo,
        token: token.into(),
        puerto,
    };
    Router::new()
        .route("/api/version", get(version))
        .route("/api/status", get(estado))
        .route("/api/models", get(modelos))
        .route("/api/models/{id}", get(modelo))
        .route("/api/chat", post(chat::chat))
        .fallback(no_existe)
        // La seguridad envuelve TODO, también las rutas que no existen: sin token, ni siquiera se sabe cuáles hay.
        .layer(middleware::from_fn_with_state(
            compartido.clone(),
            seguridad,
        ))
        .with_state(compartido)
}

// ─── Seguridad ────────────────────────────────────────────────────────────────

async fn seguridad(
    State(c): State<Compartido>,
    peticion: Request,
    siguiente: Next,
) -> Result<Response, ErrorApi> {
    let cabeceras = peticion.headers();
    if !host_permitido(cabeceras, c.puerto) {
        log::warn!("petición rechazada: Host no permitido");
        return Err(ErrorApi::new(
            StatusCode::FORBIDDEN,
            "host_not_allowed",
            "Locall solo acepta peticiones dirigidas a localhost",
        ));
    }
    if cabeceras.contains_key(header::ORIGIN) {
        log::warn!("petición rechazada: viene de una web (Origin)");
        return Err(ErrorApi::new(
            StatusCode::FORBIDDEN,
            "origin_not_allowed",
            "Locall no acepta peticiones de páginas web",
        ));
    }
    let token = cabeceras
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    match token {
        None => Err(ErrorApi::new(
            StatusCode::UNAUTHORIZED,
            "token_required",
            "falta el token (cabecera Authorization: Bearer …); está en %LOCALAPPDATA%\\Locall\\token",
        )),
        Some(t) if !token::iguales(t.trim(), &c.token) => Err(ErrorApi::new(
            StatusCode::UNAUTHORIZED,
            "token_invalid",
            "el token no es correcto",
        )),
        Some(_) => Ok(siguiente.run(peticion).await),
    }
}

/// ¿Va dirigida a nosotros? `127.0.0.1`, `localhost` o `[::1]`, sin puerto o con el nuestro.
fn host_permitido(cabeceras: &HeaderMap, puerto: u16) -> bool {
    let Some(host) = cabeceras.get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let host = host.trim().to_ascii_lowercase();
    let (nombre, su_puerto) = match host.rsplit_once(':') {
        // "[::1]:11470" o "localhost:11470" (pero no "[::1]" sin puerto, que también lleva ':').
        Some((n, p)) if !p.contains(']') => (n.to_owned(), Some(p.to_owned())),
        _ => (host.clone(), None),
    };
    let nombre_ok = matches!(nombre.as_str(), "127.0.0.1" | "localhost" | "[::1]");
    let puerto_ok = su_puerto.is_none_or(|p| p == puerto.to_string());
    nombre_ok && puerto_ok
}

// ─── Rutas ────────────────────────────────────────────────────────────────────

async fn version() -> Json<Value> {
    Json(json!({
        "name": "locall",
        "version": env!("CARGO_PKG_VERSION"),
        "engine": VERSION_MOTOR,
    }))
}

async fn estado(State(c): State<Compartido>) -> Json<Value> {
    let (estado, modelo, en_uso) = match c.nucleo.estado() {
        Estado::Vacio => ("idle", None, 0),
        Estado::Cargando { modelo } => ("loading", Some(modelo), 0),
        Estado::Listo { modelo, en_uso } => ("ready", Some(modelo), en_uso),
    };
    Json(json!({
        "state": estado,
        "model": modelo,
        "in_use": en_uso,
        "warnings": c.nucleo.avisos(),
    }))
}

async fn modelos(State(c): State<Compartido>) -> Result<Json<Value>, ErrorApi> {
    let modelos = en_hilo(move || c.nucleo.modelos()).await?;
    Ok(Json(
        json!({ "models": modelos.iter().map(modelo_json).collect::<Vec<_>>() }),
    ))
}

async fn modelo(
    State(c): State<Compartido>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ErrorApi> {
    let buscado = id.clone();
    match en_hilo(move || c.nucleo.modelo(&buscado)).await? {
        Some(m) => Ok(Json(modelo_json(&m))),
        None => Err(ErrorNucleo::NoInstalado(id).into()),
    }
}

async fn no_existe() -> ErrorApi {
    ErrorApi::new(
        StatusCode::NOT_FOUND,
        "not_found",
        "esa ruta no existe en la API de Locall",
    )
}

/// Un modelo en JSON. Nunca incluye rutas del disco.
fn modelo_json(m: &Modelo) -> Value {
    json!({
        "id": m.id.to_string(),
        "name": m.nombre,
        "source": m.origen,
        "revision": m.revision,
        "quantization": m.cuantizacion,
        "license": m.licencia,
        "architecture": m.arquitectura,
        "context_max": m.contexto_max,
        "verified": m.verificado,
        "size_bytes": m.bytes_totales(),
        "installed_at": m.instalado,
        "last_used_at": m.ultimo_uso,
        "files": m.archivos.iter().map(|a| json!({
            "name": a.nombre.to_string(),
            "size_bytes": a.bytes,
            "sha256": a.huella.hex(),
        })).collect::<Vec<_>>(),
    })
}

/// Ejecuta una función del núcleo (que puede bloquear: base de datos, disco) en un hilo aparte, para no
/// detener el servidor mientras tanto.
pub(crate) async fn en_hilo<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, ErrorNucleo> + Send + 'static,
) -> Result<T, ErrorApi> {
    match tokio::task::spawn_blocking(f).await {
        Ok(resultado) => resultado.map_err(ErrorApi::from),
        Err(_) => Err(ErrorApi::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "error interno de Locall",
        )),
    }
}

/// El código de un error del motor según su causa reconocida (ADR 0011).
fn codigo_del_motor(e: &ErrorMotor) -> (StatusCode, &'static str) {
    use StatusCode as S;
    match e.causa() {
        Causa::ContextoExcedido { .. } => (S::BAD_REQUEST, "context_exceeded"),
        Causa::MemoriaInsuficiente => (S::SERVICE_UNAVAILABLE, "insufficient_memory"),
        Causa::ModeloDanado => (S::INTERNAL_SERVER_ERROR, "model_damaged"),
        Causa::ArquitecturaNoSoportada(_) => (S::UNPROCESSABLE_ENTITY, "model_incompatible"),
        Causa::Cargando => (S::SERVICE_UNAVAILABLE, "model_loading"),
        _ if matches!(e, ErrorMotor::NoEncontrado(_)) => (S::SERVICE_UNAVAILABLE, "engine_missing"),
        _ => (S::INTERNAL_SERVER_ERROR, "engine_error"),
    }
}

// ─── Errores ──────────────────────────────────────────────────────────────────

/// Un error de la API: estado HTTP, código fijo (inglés) y mensaje (español).
#[derive(Debug)]
pub struct ErrorApi {
    estado: StatusCode,
    codigo: &'static str,
    mensaje: String,
}

impl ErrorApi {
    pub(crate) fn new(
        estado: StatusCode,
        codigo: &'static str,
        mensaje: impl Into<String>,
    ) -> ErrorApi {
        ErrorApi {
            estado,
            codigo,
            mensaje: mensaje.into(),
        }
    }
}

impl ErrorApi {
    /// `{"code": "...", "message": "..."}`
    pub(crate) fn cuerpo(&self) -> Value {
        json!({ "code": self.codigo, "message": self.mensaje })
    }
}

impl IntoResponse for ErrorApi {
    fn into_response(self) -> Response {
        let cuerpo = json!({ "error": self.cuerpo() });
        (self.estado, Json(cuerpo)).into_response()
    }
}

impl From<ErrorNucleo> for ErrorApi {
    fn from(e: ErrorNucleo) -> Self {
        use StatusCode as S;
        let (estado, codigo) = match &e {
            ErrorNucleo::NoInstalado(_) => (S::NOT_FOUND, "model_not_found"),
            ErrorNucleo::YaInstalado(_) => (S::CONFLICT, "model_exists"),
            ErrorNucleo::YaInstalandose(_) => (S::CONFLICT, "install_in_progress"),
            ErrorNucleo::EnUso(_) => (S::CONFLICT, "model_in_use"),
            ErrorNucleo::RepoInvalido(_)
            | ErrorNucleo::RevisionInvalida(_)
            | ErrorNucleo::IdInvalido(_) => (S::BAD_REQUEST, "invalid_request"),
            ErrorNucleo::RepoNoEncontrado(_) | ErrorNucleo::ArchivoNoEncontrado { .. } => {
                (S::NOT_FOUND, "not_found_on_hub")
            }
            ErrorNucleo::Protegido(_) => (S::FORBIDDEN, "gated_repo"),
            ErrorNucleo::Incompatible(_) => (S::UNPROCESSABLE_ENTITY, "model_incompatible"),
            ErrorNucleo::Cancelado => (S::CONFLICT, "cancelled"),
            ErrorNucleo::Red(_) | ErrorNucleo::HuggingFace { .. } | ErrorNucleo::RespuestaHf(_) => {
                (S::BAD_GATEWAY, "hub_unavailable")
            }
            ErrorNucleo::Carga(carga) => match carga {
                ErrorCarga::NoCabe { .. } => (S::SERVICE_UNAVAILABLE, "insufficient_memory"),
                ErrorCarga::Incompatible(_) => (S::UNPROCESSABLE_ENTITY, "model_incompatible"),
                ErrorCarga::Cabecera(_) | ErrorCarga::FaltaParte(_) => {
                    (S::INTERNAL_SERVER_ERROR, "model_damaged")
                }
                ErrorCarga::Motor(m) => codigo_del_motor(m),
                ErrorCarga::Sistema(_) => (S::INTERNAL_SERVER_ERROR, "internal_error"),
            },
            ErrorNucleo::Motor(m) => codigo_del_motor(m),
            _ => (S::INTERNAL_SERVER_ERROR, "internal_error"),
        };
        if estado.is_server_error() {
            log::warn!("error en la API: {e}");
        }
        ErrorApi::new(estado, codigo, e.to_string())
    }
}
