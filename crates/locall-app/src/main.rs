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

// ─── Órdenes: ajustes ─────────────────────────────────────────────────────────

/// Los ajustes en uso y los guardados en `config.toml` (`GET /api/settings`).
#[tauri::command]
async fn ajustes(conexion: tauri::State<'_, Conexion>) -> Result<Value, String> {
    let conexion = conexion.inner().clone();
    en_hilo(move || conexion.con_cliente(|c| c.get("/api/settings"))).await
}

/// Guarda cambios de ajustes (`{"context": 8192, …}`). Se aplican al reiniciar Locall.
#[tauri::command]
async fn guardar_ajustes(
    conexion: tauri::State<'_, Conexion>,
    cambios: Value,
) -> Result<Value, String> {
    let conexion = conexion.inner().clone();
    en_hilo(move || conexion.con_cliente(|c| c.patch("/api/settings", &cambios))).await
}

/// Cierra Locall para que se vuelva a abrir con los ajustes nuevos. No hace falta abrirlo: la próxima petición de la
/// app (la barra de estado pregunta cada 2 s) lo abre sola, igual que hace la línea de comandos.
#[tauri::command]
async fn reiniciar_locall(conexion: tauri::State<'_, Conexion>) -> Result<(), String> {
    let conexion = conexion.inner().clone();
    en_hilo(move || {
        let cliente = conexion.cliente()?;
        cliente
            .post("/api/shutdown", &json!({}))
            .map_err(|e| e.to_string())?;
        conexion.olvidar();
        // Esperar (como mucho 15 s) a que el Locall viejo se cierre de verdad: si no, la siguiente petición podría
        // llegarle a él, con los ajustes antiguos.
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

/// Abre en el Explorador de Windows la carpeta de datos de Locall (`"datos"`) o la de los modelos (`"modelos"`). La
/// ruta la da Locall, no la interfaz: la interfaz solo elige cuál de las dos.
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
        // El Explorador devuelve un código de salida distinto de 0 aunque abra bien la carpeta: no se mira.
        std::process::Command::new("explorer.exe")
            .arg(&ruta)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("no se pudo abrir el Explorador: {e}"))
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

/// Importa un `.gguf` que ya tienes en el equipo. Abre la ventana normal de Windows para elegirlo (la ruta la elige
/// la persona en esa ventana, nunca la interfaz). En cuanto se elige, envía el nombre del archivo por `canal` (para
/// mostrar «Importando…»: comprobar un modelo grande tarda unos segundos). Devuelve el modelo importado, o `null` si
/// se cerró la ventana sin elegir nada.
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

/// Busca en Hugging Face repositorios con GGUF por nombre (los más descargados primero).
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

// ─── Órdenes: conversaciones guardadas ────────────────────────────────────────

/// Las conversaciones guardadas, de la más reciente a la más antigua (sin los mensajes).
#[tauri::command]
async fn conversaciones(conexion: tauri::State<'_, Conexion>) -> Result<Value, String> {
    let conexion = conexion.inner().clone();
    en_hilo(move || {
        conexion.con_cliente(|c| Ok(c.get("/api/conversations")?["conversations"].take()))
    })
    .await
}

/// Una conversación con sus mensajes.
#[tauri::command]
async fn abrir_conversacion(
    conexion: tauri::State<'_, Conexion>,
    id: i64,
) -> Result<Value, String> {
    let conexion = conexion.inner().clone();
    en_hilo(move || conexion.con_cliente(|c| c.get(&format!("/api/conversations/{id}")))).await
}

/// Crea una conversación vacía con este título.
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

/// Cambia el título de una conversación. Devuelve la conversación cambiada.
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

/// Elimina una conversación de verdad (también del disco).
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

// ─── Órdenes: conversar ───────────────────────────────────────────────────────

/// Respuestas en marcha, con su aviso de parar (clave: el identificador que elige la interfaz).
#[derive(Default, Clone)]
struct Respuestas(Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>);

/// Escribe `contenido` en la conversación guardada `conversacion` y pide la respuesta a `modelo`. Locall envía al
/// modelo la conversación entera y guarda la pregunta y la respuesta. Cada trozo (texto, razonamiento, progreso de
/// lectura) se envía a la interfaz por `canal` en cuanto llega; al terminar devuelve el último evento (`finish`,
/// `usage`, `speed`, y la conversación y el mensaje guardados). Si se para, devuelve `finish: "cancelled"`: la
/// interfaz ya tiene el texto, y Locall guarda lo que llegó marcado como cortado.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // las órdenes de Tauri reciben cada dato de la interfaz como argumento
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
            buscar_modelos,
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
            parar
        ])
        .run(tauri::generate_context!())
        .expect("no se pudo abrir la ventana de Locall");
}
