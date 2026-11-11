//! `locall`: the program. A single executable for the core and the command line (ADR 0015):
//!
//! - `locall serve` starts the core and the local API (handled by this file, which holds the server);
//! - every other command is the CLI (`locall-cli`), a client of that API that starts it in the background when
//!   needed (ADR 0017).

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use locall_cli::{ErrorCli, Orden};
use locall_config::{Config, Rutas, registros};
use locall_core::{Nucleo, OpcionesNucleo};

fn main() -> ExitCode {
    let orden = match locall_cli::interpretar(std::env::args_os().skip(1)) {
        Ok(orden) => orden,
        Err(error) => {
            eprintln!("Error: {error}");
            return ExitCode::from(2);
        }
    };
    if orden == Orden::Servir {
        return match servir() {
            Ok(()) => ExitCode::SUCCESS,
            Err(mensaje) => {
                eprintln!("Error: {mensaje}");
                log::error!("{mensaje}");
                ExitCode::FAILURE
            }
        };
    }
    let resultado = Rutas::por_defecto()
        .map_err(|e| ErrorCli::Config(e.to_string()))
        .and_then(|rutas| {
            let ejecutable = std::env::current_exe()
                .map_err(|e| ErrorCli::Config(format!("no se encuentra locall.exe: {e}")))?;
            locall_cli::ejecutar(&orden, &rutas, &ejecutable)
        });
    match resultado {
        Ok(texto) => {
            print!("{texto}");
            ExitCode::SUCCESS
        }
        Err(error @ ErrorCli::Uso(_)) => {
            eprintln!("Error: {error}");
            ExitCode::from(2)
        }
        Err(error) => {
            eprintln!("Error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// `locall serve`.
fn servir() -> Result<(), String> {
    let rutas = Rutas::por_defecto().map_err(|e| e.to_string())?;
    Config::crear_si_no_existe(&rutas).map_err(|e| e.to_string())?;
    let (config, _) = Config::leer(&rutas.config()).map_err(|e| e.to_string())?;
    registros::iniciar(&rutas.registros(), config.registros_dias).map_err(|e| e.to_string())?;
    log::info!("Locall {} arrancando", env!("CARGO_PKG_VERSION"));

    // The port first: if another Locall is already running, exit without touching anything (neither the database nor
    // the store).
    let puerto = config.puerto;
    let cierre = config.cierre_inactividad();
    let escucha = std::net::TcpListener::bind(("127.0.0.1", puerto)).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AddrInUse {
            format!(
                "el puerto {puerto} ya está en uso: ¿hay otro Locall abierto? Puedes cambiarlo en {} (puerto)",
                rutas.config().display()
            )
        } else {
            format!("no se pudo abrir el puerto {puerto}: {e}")
        }
    })?;
    escucha
        .set_nonblocking(true) // tokio requires it
        .map_err(|e| format!("no se pudo preparar el puerto {puerto}: {e}"))?;

    let motor = ruta_del_motor();
    if !motor.is_file() {
        eprintln!(
            "Aviso: no se encuentra el motor en {}. Podrás ver y gestionar modelos, pero no conversar. \
             Indica su ruta con la variable LOCALL_MOTOR.",
            motor.display()
        );
    }
    let mut opciones = OpcionesNucleo::nuevas(rutas.clone(), motor);
    // A Hugging Face mirror (or, in tests, a fake one). The real one by default.
    if let Some(hf) = std::env::var_os("LOCALL_HUGGING_FACE").filter(|v| !v.is_empty()) {
        opciones.hugging_face = hf.to_string_lossy().into_owned();
    }
    let nucleo = Nucleo::abrir(opciones).map_err(|e| e.to_string())?;
    for aviso in nucleo.avisos() {
        eprintln!("Aviso: {aviso}");
    }
    let nucleo = Arc::new(nucleo);
    let token = locall_api::token::leer_o_crear(&rutas.token()).map_err(|e| {
        format!(
            "no se pudo preparar el token en {}: {e}",
            rutas.token().display()
        )
    })?;

    let tiempo_de_ejecucion = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2) // the API serves few local clients; heavy work runs on separate threads
        .enable_all()
        .build()
        .map_err(|e| format!("no se pudo iniciar el servidor: {e}"))?;
    tiempo_de_ejecucion.block_on(async {
        let escucha = tokio::net::TcpListener::from_std(escucha)
            .map_err(|e| format!("no se pudo preparar el puerto {puerto}: {e}"))?;
        println!(
            "Locall {} escuchando en http://127.0.0.1:{puerto}",
            env!("CARGO_PKG_VERSION")
        );
        println!("Token de la API: en {}", rutas.token().display());
        match cierre {
            Some(limite) => println!(
                "Ctrl+C para parar. Se cerrará solo tras {} min sin uso (cierre_inactividad_minutos).",
                limite.as_secs() / 60
            ),
            None => println!("Ctrl+C para parar."),
        }
        log::info!("API escuchando en 127.0.0.1:{puerto}");

        // Stops on Ctrl+C, on `locall stop` (POST /api/shutdown) or after a while without use: whichever comes first.
        let parar = Arc::new(tokio::sync::Notify::new());
        let (rutas_api, actividad) =
            locall_api::rutas_vigiladas(Arc::clone(&nucleo), token, puerto, Arc::clone(&parar));
        let aviso = Arc::clone(&parar);
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            println!();
            println!("Parando Locall…");
            aviso.notify_one();
        });
        if let Some(limite) = cierre {
            let aviso = Arc::clone(&parar);
            tokio::spawn(async move {
                locall_api::esperar_inactividad(nucleo, actividad, limite, Duration::from_secs(30)).await;
                println!("Sin uso desde hace {} min: Locall se cierra solo.", limite.as_secs() / 60);
                aviso.notify_one();
            });
        } else {
            drop(nucleo);
        }

        axum::serve(escucha, rutas_api)
            .with_graceful_shutdown(async move { parar.notified().await })
            .await
            .map_err(|e| format!("el servidor se detuvo por un error: {e}"))
    })?;
    // Finish pending tasks (the watcher holds a copy of the core): when the last copy of the core is dropped, the
    // scheduler stops the engine.
    drop(tiempo_de_ejecucion);
    log::info!("Locall parado");
    Ok(())
}

/// `LOCALL_MOTOR`, or `motor\llama-server.exe` next to `locall.exe` (where the installer will put it).
fn ruta_del_motor() -> PathBuf {
    if let Some(ruta) = std::env::var_os("LOCALL_MOTOR").filter(|v| !v.is_empty()) {
        return PathBuf::from(ruta);
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| {
            exe.parent()
                .map(|c| c.join("motor").join("llama-server.exe"))
        })
        .unwrap_or_else(|| PathBuf::from(r"motor\llama-server.exe"))
}
