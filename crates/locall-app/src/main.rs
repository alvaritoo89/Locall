//! La aplicación de escritorio de Locall (ADR 0018).
//!
//! Esta parte en Rust es **un cliente más de la API local**, como la CLI (regla 9): reutiliza su cliente y su
//! arranque bajo demanda. La interfaz (React, en `ui/`) no habla nunca con la API: le pide las cosas a esta parte
//! con `invoke(...)`. Así **el token nunca llega a JavaScript**, y la API sigue rechazando las peticiones de webs.

// En la versión final no se abre una consola junto a la ventana.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use locall_cli::{Cliente, arranque};
use locall_config::Rutas;
use serde_json::{Value, json};

/// La conexión con Locall, compartida por las órdenes de la interfaz. Se crea al primer uso (y se rehace si Locall
/// se ha cerrado entretanto).
#[derive(Default, Clone)]
struct Conexion(Arc<Mutex<Option<Cliente>>>);

impl Conexion {
    /// Un cliente que responde; si no hay, conecta (o arranca Locall en segundo plano).
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

    /// Ejecuta `f` con un cliente; si falla la conexión, reconecta una vez y lo reintenta.
    fn con_cliente<T>(&self, f: impl Fn(&Cliente) -> Result<T, String>) -> Result<T, String> {
        match f(&self.cliente()?) {
            Ok(v) => Ok(v),
            Err(_) => {
                self.olvidar();
                f(&self.cliente()?)
            }
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

/// Lo que muestra la barra de estado (y el Inicio): versión, motor, modelo cargado y memoria del equipo.
#[tauri::command]
async fn estado(conexion: tauri::State<'_, Conexion>) -> Result<Value, String> {
    let conexion = conexion.inner().clone();
    // Las peticiones son bloqueantes: van a un hilo aparte para no congelar la ventana.
    tauri::async_runtime::spawn_blocking(move || {
        conexion.con_cliente(|c| {
            let version = c.get("/api/version").map_err(|e| e.to_string())?;
            let estado = c.get("/api/status").map_err(|e| e.to_string())?;
            let sistema = c.get("/api/system").map_err(|e| e.to_string())?;
            Ok(json!({
                "version": version["version"],
                "engine": version["engine"],
                "state": estado["state"],
                "model": estado["model"],
                "inUse": estado["in_use"],
                "warnings": estado["warnings"],
                "cpu": sistema["cpu"],
                "threads": sistema["threads"],
                "memoryTotal": sistema["memory_total_bytes"],
                "memoryAvailable": sistema["memory_available_bytes"],
            }))
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

fn main() {
    tauri::Builder::default()
        .manage(Conexion::default())
        .invoke_handler(tauri::generate_handler![estado])
        .run(tauri::generate_context!())
        .expect("no se pudo abrir la ventana de Locall");
}
