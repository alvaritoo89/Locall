//! Locall's desktop app (ADR 0018).
//!
//! This Rust side is **just another client of the local API**, like the CLI (rule 9): it reuses its client and its
//! on-demand startup. The UI (React, in `ui/`) never talks to the API: it asks this side for things with
//! `invoke(...)`. So **the token never reaches JavaScript**, and the API keeps rejecting requests from websites.

// In the release build, no console window opens next to the app window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

mod ventana;

use locall_cli::modelos::codificar;
use locall_cli::{Cliente, ErrorCli, arranque};
use locall_config::Rutas;
use serde_json::{Value, json};
use tauri::ipc::Channel;

// ─── Connection to Locall ─────────────────────────────────────────────────────

/// The connection to Locall, shared by the UI commands. Created on first use (and re-created if Locall has shut
/// down in the meantime).
#[derive(Default, Clone)]
struct Conexion(Arc<Mutex<Option<Cliente>>>);

impl Conexion {
    /// A client; if there is none, connects (or starts Locall in the background).
    fn cliente(&self) -> Result<Cliente, String> {
        let mut guardado = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(c) = guardado.as_ref() {
            return Ok(c.clone());
        }
        let rutas = Rutas::por_defecto().map_err(|e| e.to_string())?;
        let cliente =
            arranque::conectar_o_arrancar(&rutas, &ruta_de_locall()?).map_err(|e| e.to_string())?;
        *guardado = Some(cliente.clone());
        Ok(cliente)
    }

    /// Forgets the client (for example, because Locall shut down from inactivity): the next use reconnects.
    fn olvidar(&self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// Runs `f` with a client. If **the connection** fails (Locall shut down), it reconnects once and retries; API errors
    /// (a model that does not exist…) are returned as-is, without retrying.
    fn con_cliente<T>(&self, f: impl Fn(&Cliente) -> Result<T, ErrorCli>) -> Result<T, String> {
        match f(&self.cliente()?) {
            Ok(v) => Ok(v),
            Err(ErrorCli::Red(_)) => {
                self.olvidar();
                f(&self.cliente()?).map_err(|e| e.to_string())
            }
            Err(e) => Err(e.to_string()),
        }
    }
}

/// `locall.exe`: `LOCALL_EXE` if set; otherwise next to this app (where the installer will put it, and where it is
/// in development: `target\debug`).
fn ruta_de_locall() -> Result<PathBuf, String> {
    if let Some(ruta) = std::env::var_os("LOCALL_EXE").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(ruta));
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let ruta = exe.with_file_name("locall.exe");
    if ruta.is_file() {
        Ok(ruta)
    } else {
        Err(format!(
            "no se encuentra locall.exe junto a la app ({})",
            ruta.display()
        ))
    }
}

/// Runs blocking work (HTTP requests) on a separate thread so the window does not freeze.
async fn en_hilo<T: Send + 'static>(
    trabajo: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(trabajo)
        .await
        .map_err(|e| e.to_string())?
}

// ─── Commands: status ─────────────────────────────────────────────────────────

/// What the status bar (and Home) shows: version, engine, loaded model, its memory and the machine's.
#[tauri::command]
async fn estado(conexion: tauri::State<'_, Conexion>) -> Result<Value, String> {
    let conexion = conexion.inner().clone();
    en_hilo(move || {
        conexion.con_cliente(|c| {
            let version = c.get("/api/version")?;
            let estado = c.get("/api/status")?;
            let sistema = c.get("/api/system")?;
            Ok(json!({
                "version": version["version"],
                "engine": version["engine"],
                "state": estado["state"],
                "model": estado["model"],
                "inUse": estado["in_use"],
                "modelMemory": estado["model_memory_bytes"],
                "unloadIn": estado["unload_in_seconds"],
                "warnings": estado["warnings"],
                "cpu": sistema["cpu"],
                "threads": sistema["threads"],
                "memoryTotal": sistema["memory_total_bytes"],
                "memoryAvailable": sistema["memory_available_bytes"],
            }))
        })
    })
    .await
}

// ─── Commands: settings ───────────────────────────────────────────────────────

/// The settings in use and the ones saved in `config.toml` (`GET /api/settings`).
#[tauri::command]
async fn ajustes(conexion: tauri::State<'_, Conexion>) -> Result<Value, String> {
    let conexion = conexion.inner().clone();
    en_hilo(move || conexion.con_cliente(|c| c.get("/api/settings"))).await
}

/// Saves settings changes (`{"context": 8192, …}`). They apply when Locall restarts.
#[tauri::command]
async fn guardar_ajustes(
    conexion: tauri::State<'_, Conexion>,
    cambios: Value,
) -> Result<Value, String> {
    let conexion = conexion.inner().clone();
    en_hilo(move || conexion.con_cliente(|c| c.patch("/api/settings", &cambios))).await
}

/// Shuts Locall down so it starts again with the new settings. There is no need to start it: the app's next request
/// (the status bar asks every 2 s) starts it, just as the command line does.
#[tauri::command]
async fn reiniciar_locall(conexion: tauri::State<'_, Conexion>) -> Result<(), String> {
    let conexion = conexion.inner().clone();
    en_hilo(move || {
        let cliente = conexion.cliente()?;
        cliente
            .post("/api/shutdown", &json!({}))
            .map_err(|e| e.to_string())?;
        conexion.olvidar();
        // Wait (at most 15 s) for the old Locall to really exit: otherwise the next request could reach it, with the old
        // settings.
        for _ in 0..150 {
            if !cliente.responde() {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Err("Locall todavía no se ha cerrado; vuelve a intentarlo en unos segundos".to_owned())
    })
    .await
}

/// Opens Locall's data folder (`"datos"`) or the models folder (`"modelos"`) in Windows File Explorer. Locall
/// provides the path, not the UI: the UI only chooses which of the two.
#[tauri::command]
async fn abrir_carpeta(conexion: tauri::State<'_, Conexion>, cual: String) -> Result<(), String> {
    let conexion = conexion.inner().clone();
    en_hilo(move || {
        let ajustes = conexion.con_cliente(|c| c.get("/api/settings"))?;
        let ruta = match cual.as_str() {
            "datos" => ajustes["data_folder"].as_str(),
            "modelos" => ajustes["active"]["models_folder"].as_str(),
            _ => None,
        }
        .map(PathBuf::from)
        .ok_or_else(|| format!("no conozco la carpeta «{cual}»"))?;
        if !ruta.is_dir() {
            return Err(format!("la carpeta {} no existe", ruta.display()));
        }
        // File Explorer returns a non-zero exit code even when it opens the folder fine: it is not checked.
        std::process::Command::new("explorer.exe")
            .arg(&ruta)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("no se pudo abrir el Explorador: {e}"))
    })
    .await
}

// ─── Commands: models ─────────────────────────────────────────────────────────

/// The installed models, with whether they fit on the machine.
#[tauri::command]
async fn modelos(conexion: tauri::State<'_, Conexion>) -> Result<Value, String> {
    let conexion = conexion.inner().clone();
    en_hilo(move || conexion.con_cliente(|c| Ok(c.get("/api/models")?["models"].take()))).await
}

/// Imports a `.gguf` you already have on the machine. Opens the standard Windows dialog to choose it (the person
/// chooses the path in that dialog, never the UI). As soon as it is chosen, sends the file name through `canal` (to
/// show «Importando…»: checking a large model takes a few seconds). Returns the imported model, or `null` if the
/// dialog was closed without choosing anything.
#[tauri::command]
async fn importar_gguf(
    conexion: tauri::State<'_, Conexion>,
    ventana: tauri::WebviewWindow,
    canal: Channel<String>,
) -> Result<Value, String> {
    let conexion = conexion.inner().clone();
    en_hilo(move || {
        let elegido = rfd::FileDialog::new()
            .set_title("Importar un modelo GGUF")
            .add_filter("Modelos GGUF", &["gguf"])
            .set_parent(&ventana)
            .pick_file();
        let Some(ruta) = elegido else {
            return Ok(Value::Null);
        };
        let nombre = ruta
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let _ = canal.send(nombre);
        conexion.con_cliente(|c| {
            c.post(
                "/api/models/import",
                &json!({"path": ruta.to_string_lossy()}),
            )
        })
    })
    .await
}

/// Deletes a model. Returns the bytes freed.
#[tauri::command]
async fn eliminar(conexion: tauri::State<'_, Conexion>, id: String) -> Result<u64, String> {
    let conexion = conexion.inner().clone();
    en_hilo(move || {
        conexion.con_cliente(|c| {
            let r = c.delete(&format!("/api/models/{}", codificar(&id)))?;
            Ok(r["freed_bytes"].as_u64().unwrap_or(0))
        })
    })
    .await
}

/// The GGUFs of a Hugging Face repository (`owner/name`).
#[tauri::command]
async fn consultar_repo(
    conexion: tauri::State<'_, Conexion>,
    repo: String,
) -> Result<Value, String> {
    let conexion = conexion.inner().clone();
    en_hilo(move || {
        let (dueno, nombre) = repo.trim().split_once('/').ok_or_else(|| {
            "se escribe así: propietario/nombre (por ejemplo, Qwen/Qwen3-4B-GGUF)".to_owned()
        })?;
        let ruta = format!(
            "/api/hub/{}/{}",
            codificar(dueno.trim()),
            codificar(nombre.trim())
        );
        conexion.con_cliente(|c| c.get(&ruta))
    })
    .await
}

/// Searches Hugging Face for repositories with GGUFs by name (most downloaded first).
#[tauri::command]
async fn buscar_modelos(
    conexion: tauri::State<'_, Conexion>,
    texto: String,
) -> Result<Value, String> {
    let conexion = conexion.inner().clone();
    en_hilo(move || {
        conexion.con_cliente(|c| {
            Ok(c.post("/api/hub/search", &json!({"query": texto, "limit": 30}))?["results"].take())
        })
    })
    .await
}

/// Installs in progress, with their cancel flag (key: `repo/file`).
#[derive(Default, Clone)]
struct Instalaciones(Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>);

/// Installs a model from Hugging Face. Each progress event is sent to the UI through `canal` as soon as it arrives;
/// when done it returns the installed model. If cancelled, the download is left halfway and resumes next time.
#[tauri::command]
async fn instalar(
    conexion: tauri::State<'_, Conexion>,
    instalaciones: tauri::State<'_, Instalaciones>,
    repo: String,
    archivo: String,
    canal: Channel<Value>,
) -> Result<Value, String> {
    let clave = format!("{repo}/{archivo}");
    let cuerpo = json!({"repo": repo, "file": archivo});
    instalar_con(
        conexion.inner(),
        instalaciones.inner(),
        clave,
        cuerpo,
        canal,
    )
    .await
}

/// Installs a catalog model by name (`qwen3-4b`). Before downloading, Locall checks that what Hugging Face offers
/// matches what is signed in the catalog. To cancel it: `cancelar_instalacion("catalogo", nombre)`.
#[tauri::command]
async fn instalar_catalogo(
    conexion: tauri::State<'_, Conexion>,
    instalaciones: tauri::State<'_, Instalaciones>,
    nombre: String,
    canal: Channel<Value>,
) -> Result<Value, String> {
    let clave = format!("catalogo/{nombre}");
    let cuerpo = json!({"name": nombre});
    instalar_con(
        conexion.inner(),
        instalaciones.inner(),
        clave,
        cuerpo,
        canal,
    )
    .await
}

/// The catalog of compatible models. First checks GitHub for a newer one (Locall does it at most once a day); if it
/// cannot (no internet…), returns the current one with a notice in `warning`.
#[tauri::command]
async fn catalogo(conexion: tauri::State<'_, Conexion>) -> Result<Value, String> {
    let conexion = conexion.inner().clone();
    en_hilo(move || {
        conexion.con_cliente(|c| match c.post("/api/catalog/update", &json!({})) {
            Ok(mut r) => Ok(r["catalog"].take()),
            Err(ErrorCli::Api { mensaje, .. }) => {
                let mut actual = c.get("/api/catalog")?;
                actual["warning"] = json!(format!(
                    "No se pudo buscar un catálogo más nuevo: {mensaje}"
                ));
                Ok(actual)
            }
            Err(e) => Err(e),
        })
    })
    .await
}

/// What installing from Hugging Face and from the catalog have in common: progress through `canal`, cancellation
/// by `clave`.
async fn instalar_con(
    conexion: &Conexion,
    instalaciones: &Instalaciones,
    clave: String,
    cuerpo: Value,
    canal: Channel<Value>,
) -> Result<Value, String> {
    let conexion = conexion.clone();
    let cancelar = Arc::new(AtomicBool::new(false));
    instalaciones
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(clave.clone(), Arc::clone(&cancelar));
    let lista = instalaciones.clone();

    let resultado = en_hilo(move || {
        let cliente = conexion.cliente()?;
        let respuesta = cliente
            .post_sin_leer("/api/models/pull", &cuerpo)
            .map_err(|e| e.to_string())?;
        for linea in BufReader::new(respuesta.into_body().into_reader()).lines() {
            if cancelar.load(Ordering::Relaxed) {
                // Dropping the response closes the connection: Locall pauses the download (it can be resumed).
                return Err("cancelada (lo descargado se conserva para continuar)".to_owned());
            }
            let linea = linea.map_err(|e| format!("la descarga se cortó: {e}"))?;
            let Ok(evento) = serde_json::from_str::<Value>(&linea) else {
                continue;
            };
            match evento["type"].as_str() {
                Some("progress") => {
                    let _ = canal.send(evento);
                }
                Some("done") => return Ok(evento["model"].clone()),
                Some("error") => {
                    return Err(evento["error"]["message"]
                        .as_str()
                        .unwrap_or("error al instalar")
                        .to_owned());
                }
                _ => {}
            }
        }
        Err("la descarga se cortó; vuelve a intentarlo para continuar".to_owned())
    })
    .await;

    lista
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&clave);
    resultado
}

// ─── Commands: saved conversations ────────────────────────────────────────────

/// The saved conversations, most recent first (without the messages).
#[tauri::command]
async fn conversaciones(conexion: tauri::State<'_, Conexion>) -> Result<Value, String> {
    let conexion = conexion.inner().clone();
    en_hilo(move || {
        conexion.con_cliente(|c| Ok(c.get("/api/conversations")?["conversations"].take()))
    })
    .await
}

/// A conversation with its messages.
#[tauri::command]
async fn abrir_conversacion(
    conexion: tauri::State<'_, Conexion>,
    id: i64,
) -> Result<Value, String> {
    let conexion = conexion.inner().clone();
    en_hilo(move || conexion.con_cliente(|c| c.get(&format!("/api/conversations/{id}")))).await
}

/// Creates an empty conversation with this title.
#[tauri::command]
async fn crear_conversacion(
    conexion: tauri::State<'_, Conexion>,
    titulo: String,
) -> Result<Value, String> {
    let conexion = conexion.inner().clone();
    en_hilo(move || {
        conexion.con_cliente(|c| c.post("/api/conversations", &json!({"title": titulo})))
    })
    .await
}

/// Changes a conversation's title. Returns the changed conversation.
#[tauri::command]
async fn renombrar_conversacion(
    conexion: tauri::State<'_, Conexion>,
    id: i64,
    titulo: String,
) -> Result<Value, String> {
    let conexion = conexion.inner().clone();
    en_hilo(move || {
        conexion.con_cliente(|c| {
            c.patch(
                &format!("/api/conversations/{id}"),
                &json!({"title": titulo}),
            )
        })
    })
    .await
}

/// Deletes a conversation for real (from disk too).
#[tauri::command]
async fn eliminar_conversacion(
    conexion: tauri::State<'_, Conexion>,
    id: i64,
) -> Result<(), String> {
    let conexion = conexion.inner().clone();
    en_hilo(move || {
        conexion.con_cliente(|c| c.delete(&format!("/api/conversations/{id}")).map(|_| ()))
    })
    .await
}

// ─── Commands: chat ───────────────────────────────────────────────────────────

/// Answers in progress, with their stop flag (key: the identifier chosen by the UI).
#[derive(Default, Clone)]
struct Respuestas(Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>);

/// Writes `contenido` into the saved conversation `conversacion` and asks `modelo` for the answer. Locall sends the
/// model the whole conversation and saves the question and the answer. Each chunk (text, reasoning, prompt-processing
/// progress) is sent to the UI through `canal` as soon as it arrives; when done it returns the last event (`finish`,
/// `usage`, `speed`, and the saved conversation and message). If stopped, it returns `finish: "cancelled"`: the UI
/// already has the text, and Locall saves what arrived marked as cut off.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri commands receive each piece of data from the UI as an argument
async fn conversar(
    conexion: tauri::State<'_, Conexion>,
    respuestas: tauri::State<'_, Respuestas>,
    peticion: String,
    conversacion: i64,
    contenido: String,
    modelo: String,
    sistema: String,
    temperatura: Option<f64>,
    canal: Channel<Value>,
) -> Result<Value, String> {
    let conexion = conexion.inner().clone();
    let parar = Arc::new(AtomicBool::new(false));
    respuestas
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(peticion.clone(), Arc::clone(&parar));
    let lista = respuestas.inner().clone();

    let resultado = en_hilo(move || {
        let cliente = conexion.cliente()?;
        let cuerpo = json!({
            "content": contenido,
            "model": modelo,
            "system": sistema,
            "temperature": temperatura,
        });
        let respuesta = cliente
            .post_sin_leer(
                &format!("/api/conversations/{conversacion}/messages"),
                &cuerpo,
            )
            .map_err(|e| e.to_string())?;
        for linea in BufReader::new(respuesta.into_body().into_reader()).lines() {
            if parar.load(Ordering::Relaxed) {
                // Dropping the response closes the connection: the engine stops generating (Phase 3).
                return Ok(json!({"finish": "cancelled"}));
            }
            let linea = linea.map_err(|e| format!("la respuesta se cortó: {e}"))?;
            let Ok(evento) = serde_json::from_str::<Value>(&linea) else {
                continue;
            };
            match evento["type"].as_str() {
                Some("done") => return Ok(evento),
                Some("error") => {
                    return Err(evento["error"]["message"]
                        .as_str()
                        .unwrap_or("error al responder")
                        .to_owned());
                }
                _ => {
                    let _ = canal.send(evento);
                }
            }
        }
        Err("la respuesta se cortó".to_owned())
    })
    .await;

    lista
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&peticion);
    resultado
}

/// Asks to stop an answer in progress.
#[tauri::command]
fn parar(respuestas: tauri::State<'_, Respuestas>, peticion: String) {
    if let Some(aviso) = respuestas
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&peticion)
    {
        aviso.store(true, Ordering::Relaxed);
    }
}

/// Copies text to the Windows clipboard (the «Copiar» button). Done by Rust rather than the embedded browser, which
/// may have the clipboard blocked.
#[tauri::command]
fn copiar(texto: String) -> Result<(), String> {
    locall_system::copiar_al_portapapeles(&texto).map_err(|e| format!("no se pudo copiar: {e}"))
}

/// Asks to cancel an install in progress.
#[tauri::command]
fn cancelar_instalacion(
    instalaciones: tauri::State<'_, Instalaciones>,
    repo: String,
    archivo: String,
) {
    if let Some(aviso) = instalaciones
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&format!("{repo}/{archivo}"))
    {
        aviso.store(true, Ordering::Relaxed);
    }
}

fn main() {
    let resultado = tauri::Builder::default()
        // The window starts hidden (tauri.conf.json) and is shown already placed where it was last time.
        .setup(|app| {
            if let Some(principal) = tauri::Manager::get_webview_window(app, "main") {
                ventana::restaurar(&principal);
            }
            Ok(())
        })
        .on_window_event(|ventana, evento| {
            if let tauri::WindowEvent::CloseRequested { .. } = evento {
                ventana::guardar(ventana);
            }
        })
        .manage(Conexion::default())
        .manage(Instalaciones::default())
        .manage(Respuestas::default())
        .invoke_handler(tauri::generate_handler![
            estado,
            modelos,
            eliminar,
            consultar_repo,
            buscar_modelos,
            catalogo,
            instalar_catalogo,
            importar_gguf,
            ajustes,
            guardar_ajustes,
            reiniciar_locall,
            abrir_carpeta,
            instalar,
            cancelar_instalacion,
            conversaciones,
            abrir_conversacion,
            crear_conversacion,
            renombrar_conversacion,
            eliminar_conversacion,
            conversar,
            parar,
            copiar
        ])
        .run(tauri::generate_context!());
    // The release build has no console: if the window cannot open (for example, because WebView2 is missing), it says
    // so with a Windows dialog instead of exiting without explanation.
    if let Err(error) = resultado {
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("Locall")
            .set_description(format!(
                "No se pudo abrir la ventana de Locall.\n\n{error}\n\nSi el problema sigue, comprueba que Windows tiene \
                 instalado Microsoft Edge WebView2."
            ))
            .show();
        std::process::exit(1);
    }
}
