//! The settings (`config.toml`), for the app's Settings screen (Phase 5e).
//!
//! - `GET /api/settings`: the settings **in use** (those read when Locall opened), the **saved** ones (what is in
//!   `config.toml` now) and whether a restart is needed to apply the difference.
//! - `PATCH /api/settings` with the ones being changed, for example `{"context": 8192, "threads": "auto"}`. Only
//!   `context`, `threads`, `unload_minutes`, `exit_minutes` and `log_days` can be changed; the models folder, the port
//!   and `parallel` are shown but can only be changed by hand in `config.toml` (ADR 0020). Answers like `GET`.
//!
//! Changes take effect **when Locall is reopened** (`POST /api/shutdown`; the app and the CLI open it by themselves).

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use locall_config::lector::Valor;
use locall_config::{Config, Hilos};
use serde_json::{Value, json};

use crate::{Compartido, ErrorApi, en_hilo};

/// Name in the API → name in `config.toml`.
const CAMPOS: [(&str, &str); 5] = [
    ("context", "contexto"),
    ("threads", "hilos"),
    ("unload_minutes", "inactividad_minutos"),
    ("exit_minutes", "cierre_inactividad_minutos"),
    ("log_days", "registros_dias"),
];

fn invalida(mensaje: impl Into<String>) -> ErrorApi {
    ErrorApi::new(StatusCode::BAD_REQUEST, "invalid_request", mensaje)
}

pub(crate) async fn ver(State(c): State<Compartido>) -> Result<Json<Value>, ErrorApi> {
    let guardados = {
        let c = c.clone();
        en_hilo(move || c.nucleo.ajustes_guardados()).await?
    };
    Ok(Json(respuesta(&c, &guardados)))
}

pub(crate) async fn cambiar(
    State(c): State<Compartido>,
    cuerpo: Bytes,
) -> Result<Json<Value>, ErrorApi> {
    let json: Value = serde_json::from_slice(&cuerpo)
        .map_err(|e| invalida(format!("el cuerpo no es JSON válido: {e}")))?;
    let objeto = json
        .as_object()
        .filter(|o| !o.is_empty())
        .ok_or_else(|| invalida("envía un objeto con los ajustes que cambian"))?;
    let mut cambios: Vec<(&'static str, Valor)> = Vec::new();
    for (campo, valor) in objeto {
        let Some((_, clave)) = CAMPOS.iter().find(|(nombre, _)| nombre == campo) else {
            return Err(invalida(format!(
                "«{campo}» no se puede cambiar desde aquí (se pueden: context, threads, unload_minutes, exit_minutes, \
                 log_days)"
            )));
        };
        let valor = match (*clave, valor) {
            ("hilos", Value::String(t)) if t == "auto" => Valor::Texto("auto".into()),
            (_, v) => Valor::Entero(
                v.as_i64()
                    .ok_or_else(|| invalida(format!("«{campo}» debe ser un número entero")))?,
            ),
        };
        cambios.push((clave, valor));
    }
    let guardados = {
        let c = c.clone();
        en_hilo(move || c.nucleo.guardar_ajustes(&cambios)).await?
    };
    Ok(Json(respuesta(&c, &guardados)))
}

fn respuesta(c: &Compartido, guardados: &Config) -> Value {
    let en_uso = c.nucleo.config();
    json!({
        "active": ajustes_json(c, en_uso),
        "saved": ajustes_json(c, guardados),
        "restart_needed": en_uso != guardados,
        "threads_available": locall_system::hilos_para_motor(),
        "data_folder": c.nucleo.rutas().datos().display().to_string(),
    })
}

fn ajustes_json(c: &Compartido, a: &Config) -> Value {
    json!({
        "context": a.contexto,
        "threads": match a.hilos {
            Hilos::Auto => json!("auto"),
            Hilos::Numero(n) => json!(n),
        },
        "unload_minutes": a.inactividad_minutos,
        "exit_minutes": a.cierre_inactividad_minutos,
        "log_days": a.registros_dias,
        "models_folder": a.carpeta_modelos(c.nucleo.rutas()).display().to_string(),
        "port": a.puerto,
        "parallel": a.paralelo,
    })
}
