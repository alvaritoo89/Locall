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

/// Responde por trozos en NDJSON (API propia). `fin_json` convierte el resultado final en el último evento.
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

/// Responde por trozos con el formato que diga `convertir` (NDJSON, SSE…). `convertir` recibe cada paso y
/// devuelve el texto que se envía al cliente, o `None` para no enviar nada por ese paso.
pub(crate) async fn transmitir<T, F>(
    mut rx: mpsc::Receiver<Paso<T>>,
    tipo: &'static str,
    convertir: F,
) -> Result<Response, ErrorApi>
where
    T: Send + 'static,
    F: FnMut(Paso<T>) -> Option<String> + Send + 'static,
{
    // El primer mensaje decide: si es un error, respuesta HTTP de error; si no, empezamos a enviar.
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
