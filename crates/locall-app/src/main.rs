//! La aplicación de escritorio de Locall (ADR 0018).
//!
//! Esta parte en Rust es **un cliente más de la API local**, como la CLI (regla 9): reutiliza su cliente y su
//! arranque bajo demanda. La interfaz (React, en `ui/`) no habla nunca con la API: le pide las cosas a esta parte
//! con `invoke(...)`. Así **el token nunca llega a JavaScript**, y la API sigue rechazando las peticiones de webs.

// En la versión final no se abre una consola junto a la ventana.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use locall_cli::modelos::codificar;
use locall_cli::{Cliente, ErrorCli, arranque};
use locall_config::Rutas;
use serde_json::{Value, json};
use tauri::ipc::Channel;

// ─── Conexión con Locall ──────────────────────────────────────────────────────

/// La conexión con Locall, compartida por las órdenes de la interfaz. Se crea al primer uso (y se rehace si Locall
/// se ha cerrado entretanto).
#[derive(Default, Clone)]
struct Conexion(Arc<Mutex<Option<Cliente>>>);

impl Conexion {
    /// Un cliente; si no hay, conecta (o arranca Locall en segundo plano).
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

    /// Olvida el cliente (por ejemplo, porque Locall se cerró por inactividad): el siguiente uso reconecta.
    fn olvidar(&self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// Ejecuta `f` con un cliente. Si falla **la conexión** (Locall se cerró), reconecta una vez y lo reintenta; los
    /// errores de la API (un modelo que no existe…) se devuelven tal cual, sin reintentar.
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

/// `locall.exe`: `LOCALL_EXE` si está definida; si no, junto a esta app (donde lo dejará el instalador, y donde está
/// en desarrollo: `target\debug`).
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

/// Ejecuta trabajo bloqueante (peticiones HTTP) en un hilo aparte, para no congelar la ventana.
async fn en_hilo<T: Send + 'static>(
    trabajo: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(trabajo)
        .await
        .map_err(|e| e.to_string())?
}

// ─── Órdenes: estado ──────────────────────────────────────────────────────────

/// Lo que muestra la barra de estado (y el Inicio): versión, motor, modelo cargado, su memoria y la del equipo.
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

// ─── Órdenes: modelos ─────────────────────────────────────────────────────────

/// Los modelos instalados, con si caben en el equipo.
#[tauri::command]
async fn modelos(conexion: tauri::State<'_, Conexion>) -> Result<Value, String> {
    let conexion = conexion.inner().clone();
    en_hilo(move || conexion.con_cliente(|c| Ok(c.get("/api/models")?["models"].take()))).await
}

/// Elimina un modelo. Devuelve los bytes liberados.
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

/// Los GGUF de un repositorio de Hugging Face (`propietario/nombre`).
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

/// Instalaciones en marcha, con su aviso de cancelar (clave: `repo/archivo`).
#[derive(Default, Clone)]
struct Instalaciones(Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>);

/// Instala un modelo de Hugging Face. Cada evento de progreso se envía a la interfaz por `canal` en cuanto llega;
/// al terminar devuelve el modelo instalado. Si se cancela, la descarga queda a medias y la próxima vez continúa.
#[tauri::command]
async fn instalar(
    conexion: tauri::State<'_, Conexion>,
    instalaciones: tauri::State<'_, Instalaciones>,
    repo: String,
    archivo: String,
    canal: Channel<Value>,
) -> Result<Value, String> {
    let conexion = conexion.inner().clone();
    let clave = format!("{repo}/{archivo}");
    let cancelar = Arc::new(AtomicBool::new(false));
    instalaciones
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(clave.clone(), Arc::clone(&cancelar));
    let lista = instalaciones.inner().clone();

    let resultado = en_hilo(move || {
        let cliente = conexion.cliente()?;
        let respuesta = cliente
            .post_sin_leer("/api/models/pull", &json!({"repo": repo, "file": archivo}))
            .map_err(|e| e.to_string())?;
        for linea in BufReader::new(respuesta.into_body().into_reader()).lines() {
            if cancelar.load(Ordering::Relaxed) {
                // Al soltar la respuesta se cierra la conexión: Locall pausa la descarga (se puede continuar).
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

// ─── Órdenes: conversar ───────────────────────────────────────────────────────

/// Respuestas en marcha, con su aviso de parar (clave: el identificador que elige la interfaz).
#[derive(Default, Clone)]
struct Respuestas(Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>);

/// Pide una respuesta a `modelo` para la conversación `mensajes` (`[{role, content}]`). Cada trozo (texto,
/// razonamiento, progreso de lectura) se envía a la interfaz por `canal` en cuanto llega; al terminar devuelve el
/// resumen (`finish`, `usage`, `speed`). Si se para, devuelve `finish: "cancelled"`: la interfaz ya tiene el texto.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // las órdenes de Tauri reciben cada dato de la interfaz como argumento
async fn conversar(
    conexion: tauri::State<'_, Conexion>,
    respuestas: tauri::State<'_, Respuestas>,
    peticion: String,
    modelo: String,
    mensajes: Vec<Value>,
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
        let mut cuerpo = json!({"model": modelo, "messages": mensajes, "stream": true});
        if let Some(t) = temperatura {
            cuerpo["options"] = json!({"temperature": t});
        }
        let respuesta = cliente
            .post_sin_leer("/api/chat", &cuerpo)
            .map_err(|e| e.to_string())?;
        for linea in BufReader::new(respuesta.into_body().into_reader()).lines() {
            if parar.load(Ordering::Relaxed) {
                // Al soltar la respuesta se cierra la conexión: el motor deja de generar (Fase 3).
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

/// Pide parar una respuesta en marcha.
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

/// Pide cancelar una instalación en marcha.
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
    tauri::Builder::default()
        .manage(Conexion::default())
        .manage(Instalaciones::default())
        .manage(Respuestas::default())
        .invoke_handler(tauri::generate_handler![
            estado,
            modelos,
            eliminar,
            consultar_repo,
            instalar,
            cancelar_instalacion,
            conversar,
            parar
        ])
        .run(tauri::generate_context!())
        .expect("no se pudo abrir la ventana de Locall");
}
