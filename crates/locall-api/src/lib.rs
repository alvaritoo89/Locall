//! # locall-api
//!
//! Locall's local API: an HTTP server (axum) that only listens on `127.0.0.1` and translates between JSON and the
//! [`Nucleo`] (ADR 0015). It contains no logic of its own: the core does everything (rule 9).
//!
//! **Security**, in this order, before any route (including those that do not exist):
//!
//! 1. **`Host`** must be `127.0.0.1`, `localhost` or `[::1]` (with our port). This blocks "DNS rebinding": a
//!    malicious website that points its domain to 127.0.0.1 to talk to programs on your computer.
//! 2. **`Origin`**: requests a browser makes from a website carry this header; all of them are rejected (the desktop
//!    app will have its origin allowed when it exists). This way no website you visit can use Locall.
//! 3. **Token** (`Authorization: Bearer …`), compared in constant time.
//!
//! Errors always have the shape `{"error": {"code": "...", "message": "..."}}`: `code` in English and fixed, for
//! programs; `message` in Spanish, for people.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::Json;
use axum::Router;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post};
use locall_core::{ErrorNucleo, Nucleo};
use locall_engine::{Causa, ErrorMotor};
use locall_registry::Modelo;
use locall_scheduler::ErrorCarga;
use locall_scheduler::Estado;
use serde_json::{Value, json};

mod ajustes;
#[cfg(test)]
mod basura;
mod catalogo;
mod chat;
mod conversaciones;
mod flujo;
mod modelos;
mod openai;
pub mod token;

/// Engine version this version of Locall was tested with.
pub const VERSION_MOTOR: &str = "llama.cpp b12189";

// ─── Construction ─────────────────────────────────────────────────────────────

/// What all the requests share.
#[derive(Clone)]
struct Compartido {
    nucleo: Arc<Nucleo>,
    token: Arc<str>,
    puerto: u16,
    actividad: Actividad,
    /// Tells the program it must shut down (`POST /api/shutdown`, the `locall stop` command).
    parar: Arc<tokio::sync::Notify>,
}

/// When Locall was last used (for the idle shutdown).
#[derive(Clone, Debug)]
pub struct Actividad(Arc<Mutex<Instant>>);

impl Actividad {
    pub fn nueva() -> Actividad {
        Actividad(Arc::new(Mutex::new(Instant::now())))
    }

    /// Records that it is being used now.
    pub fn tocar(&self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Instant::now();
    }

    /// Time since the last use.
    pub fn inactivo(&self) -> Duration {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .elapsed()
    }
}

/// Finishes when Locall has gone `limite` without use: no requests and no work in progress (a long download or
/// answer counts as use even if no new requests arrive). Checked every `intervalo`.
pub async fn esperar_inactividad(
    nucleo: Arc<Nucleo>,
    actividad: Actividad,
    limite: Duration,
    intervalo: Duration,
) {
    loop {
        tokio::time::sleep(intervalo).await;
        if nucleo.ocupado() {
            actividad.tocar();
        } else if actividad.inactivo() >= limite {
            log::info!("{} s sin uso: Locall se cierra solo", limite.as_secs());
            return;
        }
    }
}

/// Builds the API routes with their security. `puerto` is the one being listened on (to validate `Host`).
pub fn rutas(nucleo: Arc<Nucleo>, token: String, puerto: u16) -> Router {
    rutas_vigiladas(nucleo, token, puerto, Arc::new(tokio::sync::Notify::new())).0
}

/// Like [`rutas`], also returning the [`Actividad`] for the idle shutdown. `parar` receives the
/// `POST /api/shutdown` notice.
pub fn rutas_vigiladas(
    nucleo: Arc<Nucleo>,
    token: String,
    puerto: u16,
    parar: Arc<tokio::sync::Notify>,
) -> (Router, Actividad) {
    let actividad = Actividad::nueva();
    let compartido = Compartido {
        nucleo,
        token: token.into(),
        puerto,
        actividad: actividad.clone(),
        parar,
    };
    let rutas = Router::new()
        .route("/api/version", get(version))
        .route("/api/system", get(sistema))
        .route("/api/status", get(estado))
        .route("/api/shutdown", post(apagar))
        .route(
            "/api/settings",
            get(ajustes::ver).merge(patch(ajustes::cambiar)),
        )
        .route("/api/models", get(modelos))
        .route(
            "/api/models/{id}",
            get(modelo).merge(delete(modelos::eliminar)),
        )
        .route("/api/models/pull", post(modelos::instalar))
        .route("/api/models/import", post(modelos::importar))
        .route("/api/catalog", get(catalogo::ver))
        .route("/api/catalog/update", post(catalogo::actualizar))
        .route("/api/hub/search", post(modelos::buscar))
        .route("/api/hub/{owner}/{name}", get(modelos::hub))
        .route("/api/chat", post(chat::chat))
        .route(
            "/api/conversations",
            get(conversaciones::listar).merge(post(conversaciones::crear)),
        )
        .route(
            "/api/conversations/{id}",
            get(conversaciones::abrir)
                .merge(patch(conversaciones::renombrar))
                .merge(delete(conversaciones::eliminar)),
        )
        .route(
            "/api/conversations/{id}/messages",
            post(conversaciones::escribir),
        )
        // OpenAI-compatible (ADR 0016).
        .route("/v1/models", get(openai::modelos))
        .route("/v1/chat/completions", post(openai::completar))
        .fallback(no_existe)
        // Security wraps EVERYTHING, including routes that do not exist: without a token, you cannot even know which exist.
        .layer(middleware::from_fn_with_state(
            compartido.clone(),
            seguridad,
        ))
        .with_state(compartido);
    (rutas, actividad)
}

// ─── Security ─────────────────────────────────────────────────────────────────

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
        Some(_) => {
            c.actividad.tocar();
            Ok(siguiente.run(peticion).await)
        }
    }
}

/// Is it addressed to us? `127.0.0.1`, `localhost` or `[::1]`, with no port or with ours.
fn host_permitido(cabeceras: &HeaderMap, puerto: u16) -> bool {
    let Some(host) = cabeceras.get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let host = host.trim().to_ascii_lowercase();
    let (nombre, su_puerto) = match host.rsplit_once(':') {
        // "[::1]:11470" or "localhost:11470" (but not "[::1]" without a port, which also contains ':').
        Some((n, p)) if !p.contains(']') => (n.to_owned(), Some(p.to_owned())),
        _ => (host.clone(), None),
    };
    let nombre_ok = matches!(nombre.as_str(), "127.0.0.1" | "localhost" | "[::1]");
    let puerto_ok = su_puerto.is_none_or(|p| p == puerto.to_string());
    nombre_ok && puerto_ok
}

// ─── Routes ───────────────────────────────────────────────────────────────────

/// Computer data (for the app's memory rule). Nothing leaves the computer: only whoever has the token asks for it.
async fn sistema() -> Result<Json<Value>, ErrorApi> {
    let datos =
        tokio::task::spawn_blocking(|| (locall_system::procesador(), locall_system::memoria()))
            .await
            .map_err(|_| crate::flujo::error_interno())?;
    let (procesador, memoria) = datos;
    let memoria = memoria.map_err(|e| {
        ErrorApi::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            format!("no se pudo consultar la memoria: {e}"),
        )
    })?;
    Ok(Json(json!({
        "cpu": procesador.nombre,
        "threads": procesador.hilos_logicos,
        "memory_total_bytes": memoria.total,
        "memory_available_bytes": memoria.disponible,
    })))
}

async fn version() -> Json<Value> {
    Json(json!({
        "name": "locall",
        "version": env!("CARGO_PKG_VERSION"),
        "engine": VERSION_MOTOR,
    }))
}

/// Asks the program to shut down (in an orderly way: it finishes the answers in progress and stops the engine).
async fn apagar(State(c): State<Compartido>) -> (StatusCode, Json<Value>) {
    log::info!("cierre pedido por la API");
    c.parar.notify_one();
    (StatusCode::ACCEPTED, Json(json!({"stopping": true})))
}

async fn estado(State(c): State<Compartido>) -> Json<Value> {
    let (estado, modelo, en_uso) = match c.nucleo.estado() {
        Estado::Vacio => ("idle", None, 0),
        Estado::Cargando { modelo } => ("loading", Some(modelo), 0),
        Estado::Listo { modelo, en_uso } => ("ready", Some(modelo), en_uso),
    };
    // Model memory and countdown: fast (a system query and a lock), no separate thread.
    let memoria = c.nucleo.memoria_del_modelo();
    let se_descarga_en = c.nucleo.se_descarga_en().map(|d| d.as_secs());
    Json(json!({
        "state": estado,
        "model": modelo,
        "in_use": en_uso,
        "model_memory_bytes": memoria,
        "unload_in_seconds": se_descarga_en,
        "warnings": c.nucleo.avisos(),
    }))
}

async fn modelos(State(c): State<Compartido>) -> Result<Json<Value>, ErrorApi> {
    let modelos = en_hilo(move || {
        let modelos = c.nucleo.modelos()?;
        Ok(modelos
            .iter()
            .map(|m| con_evaluacion(modelo_json(m), &c.nucleo, &m.id.to_string()))
            .collect::<Vec<_>>())
    })
    .await?;
    Ok(Json(json!({ "models": modelos })))
}

/// `comfortable`, `fits`, `tight` or `too_big`.
pub(crate) fn encaje_texto(encaje: locall_compat::Encaje) -> &'static str {
    match encaje {
        locall_compat::Encaje::Holgado => "comfortable",
        locall_compat::Encaje::Cabe => "fits",
        locall_compat::Encaje::Justo => "tight",
        locall_compat::Encaje::NoCabe => "too_big",
    }
}

/// Adds to a model whether it fits on the computer (`fit`), the estimated memory and whether the engine can use it.
/// If it cannot be evaluated (for example, the file was moved), those fields are `null` and the list continues.
fn con_evaluacion(mut json: Value, nucleo: &Nucleo, id: &str) -> Value {
    match nucleo.evaluar(id) {
        Ok(e) => {
            json["fit"] = json!(encaje_texto(e.encaje));
            json["memory_estimate_bytes"] = json!(e.estimacion.total);
            json["compatible"] = json!(e.comprobacion.es_compatible());
        }
        Err(error) => {
            log::warn!("no se pudo evaluar «{id}»: {error}");
            json["fit"] = Value::Null;
            json["memory_estimate_bytes"] = Value::Null;
            json["compatible"] = Value::Null;
        }
    }
    json
}

async fn modelo(
    State(c): State<Compartido>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ErrorApi> {
    let buscado = id.clone();
    let encontrado = en_hilo(move || {
        Ok(c.nucleo
            .modelo(&buscado)?
            .map(|m| con_evaluacion(modelo_json(&m), &c.nucleo, &buscado)))
    })
    .await?;
    encontrado
        .map(Json)
        .ok_or_else(|| ErrorNucleo::NoInstalado(id).into())
}

async fn no_existe() -> ErrorApi {
    ErrorApi::new(
        StatusCode::NOT_FOUND,
        "not_found",
        "esa ruta no existe en la API de Locall",
    )
}

/// A model as JSON. It never includes disk paths.
pub(crate) fn modelo_json(m: &Modelo) -> Value {
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

/// Runs a core function (which may block: database, disk) on a separate thread, so the server is not held up
/// meanwhile.
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

/// The code of an engine error according to its recognized cause (ADR 0011).
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

// ─── Errors ───────────────────────────────────────────────────────────────────

/// An API error: HTTP status, fixed code (English) and message (Spanish).
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
            ErrorNucleo::ConversacionNoEncontrada(_) => (S::NOT_FOUND, "conversation_not_found"),
            ErrorNucleo::YaInstalado(_) => (S::CONFLICT, "model_exists"),
            ErrorNucleo::YaInstalandose(_) => (S::CONFLICT, "install_in_progress"),
            ErrorNucleo::EnUso(_) => (S::CONFLICT, "model_in_use"),
            ErrorNucleo::RepoInvalido(_)
            | ErrorNucleo::RevisionInvalida(_)
            | ErrorNucleo::IdInvalido(_)
            | ErrorNucleo::BusquedaInvalida(_)
            | ErrorNucleo::AjusteInvalido(_) => (S::BAD_REQUEST, "invalid_request"),
            ErrorNucleo::NoEnCatalogo(_) => (S::NOT_FOUND, "not_in_catalog"),
            ErrorNucleo::CatalogoNoCoincide(_) => (S::CONFLICT, "catalog_mismatch"),
            ErrorNucleo::Catalogo(_) => (S::BAD_GATEWAY, "catalog_invalid"),
            ErrorNucleo::CatalogoNoDisponible(_) => (S::BAD_GATEWAY, "catalog_unavailable"),
            ErrorNucleo::RepoNoEncontrado(_) | ErrorNucleo::ArchivoNoEncontrado { .. } => {
                (S::NOT_FOUND, "not_found_on_hub")
            }
            ErrorNucleo::Protegido(_) => (S::FORBIDDEN, "gated_repo"),
            ErrorNucleo::Incompatible(_) => (S::UNPROCESSABLE_ENTITY, "model_incompatible"),
            ErrorNucleo::Cabecera(_) => (S::UNPROCESSABLE_ENTITY, "invalid_model_file"),
            ErrorNucleo::FaltaParte(_) => (S::UNPROCESSABLE_ENTITY, "model_parts_missing"),
            ErrorNucleo::SinHuella(_) => (S::UNPROCESSABLE_ENTITY, "unverifiable_file"),
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
