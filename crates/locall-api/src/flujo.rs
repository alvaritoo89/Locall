//! **Chunked** responses (NDJSON) for long core jobs: chatting, installing a model…
//!
//! The job runs on a separate thread (the core uses normal threads) and sends its events through a channel. The
//! server forwards them to the client, one JSON line per event.
//!
//! - If the client **disconnects**, the channel receiver is dropped; a watcher detects it and raises the `cancelar`
//!   flag, which the core checks (also while waiting for its turn or between chunks of a download).
//! - If the job fails **before the first event**, a normal HTTP error is returned; if it fails halfway, an event
//!   `{"type": "error", "error": {...}}`.

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::body::Body;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use locall_core::ErrorNucleo;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::ErrorApi;

/// What the job thread sends to the server.
pub(crate) enum Paso<T> {
    Evento(Value),
    Fin(Result<T, ErrorNucleo>),
}

/// A channel for the job and its cancel flag, with the disconnection watcher already running.
pub(crate) fn canal<T: Send + 'static>() -> (Emisor<T>, mpsc::Receiver<Paso<T>>) {
    let (tx, rx) = mpsc::channel::<Paso<T>>(64);
    let cancelar = Arc::new(AtomicBool::new(false));
    let vigilado = tx.clone();
    let bandera = Arc::clone(&cancelar);
    tokio::spawn(async move {
        vigilado.closed().await; // the client has gone (or the response already finished)
        bandera.store(true, Ordering::Relaxed);
    });
    (Emisor { tx, cancelar }, rx)
}

/// The job's end: sends events and knows whether it must cancel.
pub(crate) struct Emisor<T> {
    tx: mpsc::Sender<Paso<T>>,
    pub cancelar: Arc<AtomicBool>,
}

impl<T> Emisor<T> {
    /// Sends an event (from the job thread). If the client has gone, raises the cancel flag.
    pub(crate) fn evento(&self, json: Value) {
        if self.tx.blocking_send(Paso::Evento(json)).is_err() {
            self.cancelar.store(true, Ordering::Relaxed);
        }
    }

    /// Sends the final result.
    pub(crate) fn fin(self, resultado: Result<T, ErrorNucleo>) {
        let _ = self.tx.blocking_send(Paso::Fin(resultado));
    }
}

/// Waits for the end ignoring the events (for `"stream": false`).
pub(crate) async fn esperar_fin<T>(mut rx: mpsc::Receiver<Paso<T>>) -> Result<T, ErrorApi> {
    loop {
        match rx.recv().await {
            Some(Paso::Evento(_)) => {}
            Some(Paso::Fin(resultado)) => return resultado.map_err(ErrorApi::from),
            None => return Err(error_interno()),
        }
    }
}

/// Responds in chunks as NDJSON (our own API). `fin_json` turns the final result into the last event.
pub(crate) async fn en_trozos<T: Send + 'static>(
    rx: mpsc::Receiver<Paso<T>>,
    fin_json: fn(&T) -> Value,
) -> Result<Response, ErrorApi> {
    transmitir(rx, "application/x-ndjson; charset=utf-8", move |paso| {
        let linea = match paso {
            Paso::Evento(json) => json,
            Paso::Fin(Ok(valor)) => fin_json(&valor),
            Paso::Fin(Err(e)) => json!({"type": "error", "error": ErrorApi::from(e).cuerpo()}),
        };
        Some(format!(
            "{linea}
"
        ))
    })
    .await
}

/// Responds in chunks with the format `convertir` gives (NDJSON, SSE…). `convertir` receives each step and returns
/// the text sent to the client, or `None` to send nothing for that step.
pub(crate) async fn transmitir<T, F>(
    mut rx: mpsc::Receiver<Paso<T>>,
    tipo: &'static str,
    convertir: F,
) -> Result<Response, ErrorApi>
where
    T: Send + 'static,
    F: FnMut(Paso<T>) -> Option<String> + Send + 'static,
{
    // The first message decides: if it is an error, an HTTP error response; otherwise, we start sending.
    let primero = match rx.recv().await {
        Some(Paso::Fin(Err(e))) => return Err(e.into()),
        Some(paso) => paso,
        None => return Err(error_interno()),
    };
    let flujo = futures_util::stream::unfold(
        (Some(primero), rx, convertir, false),
        |(mut pendiente, mut rx, mut convertir, terminado)| async move {
            if terminado {
                return None;
            }
            loop {
                let paso = match pendiente.take() {
                    Some(p) => p,
                    None => rx.recv().await?,
                };
                let es_fin = matches!(paso, Paso::Fin(_));
                if let Some(texto) = convertir(paso) {
                    return Some((Ok::<_, Infallible>(texto), (None, rx, convertir, es_fin)));
                }
                if es_fin {
                    return None;
                }
            }
        },
    );
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, tipo)],
        Body::from_stream(flujo),
    )
        .into_response())
}

pub(crate) fn error_interno() -> ErrorApi {
    ErrorApi::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "error interno de Locall",
    )
}
