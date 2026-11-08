//! Respuestas **por trozos** (NDJSON) para trabajos largos del núcleo: conversar, instalar un modelo…
//!
//! El trabajo se ejecuta en un hilo aparte (el núcleo es de hilos normales) y manda sus eventos por un canal. El
//! servidor los va enviando al cliente, una línea JSON por evento.
//!
//! - Si el cliente **se desconecta**, se suelta el receptor del canal; un vigilante lo detecta y levanta la bandera
//!   `cancelar`, que el núcleo consulta (también mientras espera turno o entre trozos de una descarga).
//! - Si el trabajo falla **antes del primer evento**, se responde con un error HTTP normal; si falla a mitad, con un
//!   evento `{"type": "error", "error": {...}}`.

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

/// Lo que el hilo del trabajo envía al servidor.
pub(crate) enum Paso<T> {
    Evento(Value),
    Fin(Result<T, ErrorNucleo>),
}

/// Un canal para el trabajo y su bandera de cancelar, con el vigilante de desconexión ya en marcha.
pub(crate) fn canal<T: Send + 'static>() -> (Emisor<T>, mpsc::Receiver<Paso<T>>) {
    let (tx, rx) = mpsc::channel::<Paso<T>>(64);
    let cancelar = Arc::new(AtomicBool::new(false));
    let vigilado = tx.clone();
    let bandera = Arc::clone(&cancelar);
    tokio::spawn(async move {
        vigilado.closed().await; // el cliente se ha ido (o la respuesta ya terminó)
        bandera.store(true, Ordering::Relaxed);
    });
    (Emisor { tx, cancelar }, rx)
}

/// El extremo del trabajo: envía eventos y sabe si hay que cancelar.
pub(crate) struct Emisor<T> {
    tx: mpsc::Sender<Paso<T>>,
    pub cancelar: Arc<AtomicBool>,
}

impl<T> Emisor<T> {
    /// Envía un evento (desde el hilo del trabajo). Si el cliente se ha ido, levanta la bandera de cancelar.
    pub(crate) fn evento(&self, json: Value) {
        if self.tx.blocking_send(Paso::Evento(json)).is_err() {
            self.cancelar.store(true, Ordering::Relaxed);
        }
    }

    /// Envía el resultado final.
    pub(crate) fn fin(self, resultado: Result<T, ErrorNucleo>) {
        let _ = self.tx.blocking_send(Paso::Fin(resultado));
    }
}

/// Espera al final ignorando los eventos (para `"stream": false`).
pub(crate) async fn esperar_fin<T>(mut rx: mpsc::Receiver<Paso<T>>) -> Result<T, ErrorApi> {
    loop {
        match rx.recv().await {
            Some(Paso::Evento(_)) => {}
            Some(Paso::Fin(resultado)) => return resultado.map_err(ErrorApi::from),
            None => return Err(error_interno()),
        }
    }
}

/// Responde por trozos. `fin_json` convierte el resultado final en el último evento.
pub(crate) async fn en_trozos<T: Send + 'static>(
    mut rx: mpsc::Receiver<Paso<T>>,
    fin_json: fn(&T) -> Value,
) -> Result<Response, ErrorApi> {
    // El primer mensaje decide: si es un error, respuesta HTTP de error; si no, empezamos a enviar.
    let primero = match rx.recv().await {
        Some(Paso::Fin(Err(e))) => return Err(e.into()),
        Some(paso) => paso,
        None => return Err(error_interno()),
    };
    let flujo = futures_util::stream::unfold(
        (Some(primero), rx, false),
        move |(pendiente, mut rx, terminado)| async move {
            if terminado {
                return None;
            }
            let paso = match pendiente {
                Some(p) => p,
                None => rx.recv().await?,
            };
            let (linea, terminado) = match paso {
                Paso::Evento(json) => (json, false),
                Paso::Fin(Ok(valor)) => (fin_json(&valor), true),
                Paso::Fin(Err(e)) => {
                    let error = ErrorApi::from(e);
                    (json!({"type": "error", "error": error.cuerpo()}), true)
                }
            };
            Some((
                Ok::<_, Infallible>(format!("{linea}\n")),
                (None, rx, terminado),
            ))
        },
    );
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/x-ndjson; charset=utf-8")],
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
