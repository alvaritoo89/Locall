//! # locall-engine
//!
//! Manages the inference engine, llama.cpp's `llama-server` (ADR 0003), as a **separate process**:
//!
//! - Starts it with the Phase 0 security rules (ADR 0006): only `127.0.0.1`, a new port on every start, a random
//!   key passed through an **environment variable** (never as an argument, which any program can read), CORS limited
//!   to `localhost` and without the built-in web UI.
//! - Puts it in a Windows **Job Object**: if Locall dies, even abruptly, Windows kills the engine.
//! - Waits for the model to be loaded and detects whether the engine dies while loading.
//! - Always stops it when dropped (`Drop`).
//!
//! This crate does not decide which model to load or when: that is the scheduler (rule 9).

use std::ffi::OsString;
use std::fmt;
use std::fs::File;
use std::io;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use locall_system::{HilosInvalidos, Trabajo};

#[cfg(test)]
mod basura;
mod chat;
mod errores;
#[cfg(test)]
mod pruebas;

pub use chat::{Evento, Fin, Mensaje, PeticionChat, Resumen, Rol};
pub use errores::Causa;

/// Attempts to start if another program takes the chosen port.
pub const INTENTOS_PUERTO: u32 = 3;

/// How often the engine is asked whether it is ready yet.
const INTERVALO_SALUD: Duration = Duration::from_millis(250);

// ─── Configuration ────────────────────────────────────────────────────────────

/// What to start the engine with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigMotor {
    /// Path of `llama-server.exe`. The folder chooses the engine type (CPU or Vulkan, D6).
    pub ejecutable: PathBuf,
    /// Model path (in a split model, the first part).
    pub modelo: PathBuf,
    /// Context size in tokens.
    pub contexto: u32,
    /// Processor threads (validated against the computer's, D5).
    pub hilos: usize,
    /// Layers offloaded to the GPU. 0 = processor only.
    pub capas_gpu: u32,
    /// Conversations the engine serves at once ("slots"). Locall uses [`PARALELO_POR_DEFECTO`] = 1 (ADR 0012): each
    /// conversation gets the whole context and the full speed; the others wait their turn.
    pub paralelo: u32,
    /// File where the engine's messages are kept (overwritten on every start).
    pub registro: PathBuf,
}

/// Default simultaneous conversations (ADR 0012).
pub const PARALELO_POR_DEFECTO: u32 = 1;

/// `llama-server` arguments. **The key is not included**: it goes through an environment variable (D3).
pub fn argumentos(config: &ConfigMotor, puerto: u16) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();
    let mut par = |nombre: &str, valor: OsString| {
        args.push(nombre.into());
        args.push(valor);
    };
    par("--model", config.modelo.clone().into());
    par("--host", "127.0.0.1".into()); // D4: this computer only
    par("--port", puerto.to_string().into());
    par("--load-mode", "none".into()); // D2: no memory mapping
    par("--threads", config.hilos.to_string().into()); // D5
    par("--ctx-size", config.contexto.to_string().into());
    par("--n-gpu-layers", config.capas_gpu.to_string().into());
    par("--parallel", config.paralelo.to_string().into());
    par("--cors-origins", "localhost".into()); // D4: no foreign websites
    args.push("--no-webui".into());
    args
}

// ─── Errors ───────────────────────────────────────────────────────────────────

/// What can go wrong with the engine.
#[derive(Debug)]
pub enum ErrorMotor {
    /// The executable or the model does not exist.
    NoEncontrado(PathBuf),
    /// A [`ConfigMotor`] setting is not valid.
    ConfigInvalida(String),
    /// The number of threads is not possible on this computer.
    Hilos(HilosInvalidos),
    /// The random key could not be generated.
    Clave(String),
    /// System error (launching the process, creating the log, the Job Object…).
    Io { contexto: String, error: io::Error },
    /// The engine exited before being ready. `causa` is what was recognized in its log.
    TerminoAlCargar {
        codigo: Option<i32>,
        causa: Causa,
        registro: PathBuf,
    },
    /// The engine was not ready within the maximum time.
    TiempoAgotado { segundos: u64 },
    /// The wait was cancelled (the engine has been stopped).
    Cancelado,
    /// Could not talk to the engine (connection refused or dropped).
    Conexion(String),
    /// The engine answered something we do not understand.
    Protocolo(String),
    /// The engine rejected the request. `tipo` and `mensaje` are its own: the message may contain private data (paths),
    /// so it is only kept for diagnostics and is **not** shown to the user.
    Respuesta {
        estado: u16,
        tipo: String,
        mensaje: String,
    },
}

impl fmt::Display for ErrorMotor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorMotor::NoEncontrado(ruta) => write!(f, "no se encuentra {}", ruta.display()),
            ErrorMotor::ConfigInvalida(e) => write!(f, "configuración del motor no válida: {e}"),
            ErrorMotor::Hilos(e) => write!(f, "{e}"),
            ErrorMotor::Clave(e) => write!(f, "no se pudo generar la clave del motor: {e}"),
            ErrorMotor::Io { contexto, error } => write!(f, "no se pudo {contexto}: {error}"),
            ErrorMotor::TerminoAlCargar {
                codigo,
                causa,
                registro,
            } => {
                if let Some(mensaje) = causa.mensaje() {
                    return write!(f, "{mensaje}");
                }
                write!(f, "el motor se cerró mientras cargaba el modelo")?;
                if let Some(c) = codigo {
                    write!(f, " (código {c})")?;
                }
                write!(f, "; detalles en {}", registro.display())
            }
            ErrorMotor::TiempoAgotado { segundos } => {
                write!(f, "el modelo no terminó de cargar en {segundos} s")
            }
            ErrorMotor::Cancelado => write!(f, "carga cancelada"),
            ErrorMotor::Conexion(e) => write!(f, "no se pudo comunicar con el motor: {e}"),
            ErrorMotor::Protocolo(e) => write!(f, "respuesta del motor no válida: {e}"),
            ErrorMotor::Respuesta {
                estado,
                tipo,
                mensaje,
            } => match errores::causa_de_respuesta(*estado, tipo, mensaje).mensaje() {
                Some(texto) => write!(f, "{texto}"),
                // Without the engine's message (D4): it could carry the model path.
                None => write!(f, "el motor rechazó la petición ({estado}, {tipo})"),
            },
        }
    }
}

impl std::error::Error for ErrorMotor {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ErrorMotor::Hilos(e) => Some(e),
            ErrorMotor::Io { error, .. } => Some(error),
            _ => None,
        }
    }
}

impl ErrorMotor {
    /// The recognized cause, so the app or the API can decide what to offer (retry, lower the context…).
    pub fn causa(&self) -> Causa {
        match self {
            ErrorMotor::TerminoAlCargar { causa, .. } => causa.clone(),
            ErrorMotor::Respuesta {
                estado,
                tipo,
                mensaje,
            } => errores::causa_de_respuesta(*estado, tipo, mensaje),
            _ => Causa::Desconocida,
        }
    }
}

/// Adds context to I/O errors (as in `locall-store`).
trait ConContexto<T> {
    fn contexto(self, contexto: impl FnOnce() -> String) -> Result<T, ErrorMotor>;
}

impl<T> ConContexto<T> for io::Result<T> {
    fn contexto(self, contexto: impl FnOnce() -> String) -> Result<T, ErrorMotor> {
        self.map_err(|error| ErrorMotor::Io {
            contexto: contexto(),
            error,
        })
    }
}

// ─── The engine ───────────────────────────────────────────────────────────────

/// A running `llama-server`. Dropping it stops it.
pub struct Motor {
    proceso: Child,
    puerto: u16,
    clave: String,
    registro: PathBuf,
    agente: ureq::Agent,
    /// Must live as long as the process: when it closes, Windows kills the engine.
    _trabajo: Trabajo,
}

impl Motor {
    /// Starts the engine and waits for the model to load. Returns the engine and how long loading took.
    ///
    /// If another program takes the port just before the engine (rare, but possible), it retries with another port, up
    /// to [`INTENTOS_PUERTO`] times.
    pub fn arrancar_listo(
        config: &ConfigMotor,
        espera_maxima: Duration,
        cancelar: &AtomicBool,
    ) -> Result<(Motor, Duration), ErrorMotor> {
        Motor::arrancar_listo_con(config, espera_maxima, cancelar, puerto_libre)
    }

    /// Like [`Motor::arrancar_listo`], choosing the port with `elegir_puerto` (so the retry can be tested).
    pub(crate) fn arrancar_listo_con(
        config: &ConfigMotor,
        espera_maxima: Duration,
        cancelar: &AtomicBool,
        mut elegir_puerto: impl FnMut() -> Result<u16, ErrorMotor>,
    ) -> Result<(Motor, Duration), ErrorMotor> {
        let mut intento = 1;
        loop {
            let mut motor = Motor::arrancar_en(config, elegir_puerto()?)?;
            match motor.esperar_listo(espera_maxima, cancelar) {
                Ok(tiempo) => return Ok((motor, tiempo)),
                Err(ErrorMotor::TerminoAlCargar {
                    causa: Causa::PuertoOcupado,
                    ..
                }) if intento < INTENTOS_PUERTO => intento += 1,
                Err(e) => return Err(e),
            }
        }
    }

    /// Launches the engine. Returns right away: to wait for the model to load, use [`Motor::esperar_listo`] (or both
    /// together, with a retry if the port gets taken: [`Motor::arrancar_listo`]).
    pub fn arrancar(config: &ConfigMotor) -> Result<Motor, ErrorMotor> {
        Motor::arrancar_en(config, puerto_libre()?)
    }

    pub(crate) fn arrancar_en(config: &ConfigMotor, puerto: u16) -> Result<Motor, ErrorMotor> {
        for ruta in [&config.ejecutable, &config.modelo] {
            if !ruta.is_file() {
                return Err(ErrorMotor::NoEncontrado(ruta.clone()));
            }
        }
        locall_system::validar_hilos(config.hilos).map_err(ErrorMotor::Hilos)?;
        if config.paralelo == 0 {
            return Err(ErrorMotor::ConfigInvalida(
                "el motor debe atender al menos 1 conversación a la vez".into(),
            ));
        }

        let clave = generar_clave()?;

        // The job is created BEFORE launching the engine: if it cannot be, we do not start.
        let trabajo = Trabajo::nuevo().contexto(|| "crear el Job Object".into())?;

        let registro = File::create(&config.registro)
            .contexto(|| format!("crear el registro {}", config.registro.display()))?;
        let registro_errores = registro
            .try_clone()
            .contexto(|| "duplicar el registro".into())?;

        let mut proceso = Command::new(&config.ejecutable)
            .args(argumentos(config, puerto))
            .env("LLAMA_API_KEY", &clave) // D3: the key NEVER as an argument
            .stdin(Stdio::null())
            .stdout(Stdio::from(registro))
            .stderr(Stdio::from(registro_errores))
            .spawn()
            .contexto(|| format!("lanzar {}", config.ejecutable.display()))?;

        // If it cannot be put in the job, it is stopped instead of being left loose.
        if let Err(error) = trabajo.asignar(&proceso) {
            let _ = proceso.kill();
            let _ = proceso.wait();
            return Err(ErrorMotor::Io {
                contexto: "meter el motor en el Job Object".into(),
                error,
            });
        }

        let agente = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .http_status_as_error(false) // a 503 ("loading") is a normal response
                .build(),
        );

        log::info!(
            "motor arrancado (proceso {}, puerto {puerto})",
            proceso.id()
        );
        Ok(Motor {
            proceso,
            puerto,
            clave,
            registro: config.registro.clone(),
            agente,
            _trabajo: trabajo,
        })
    }

    /// Waits for the model to be loaded. Returns how long it took.
    ///
    /// If the engine dies, the time runs out or it is cancelled, the engine is left stopped.
    pub fn esperar_listo(
        &mut self,
        espera_maxima: Duration,
        cancelar: &AtomicBool,
    ) -> Result<Duration, ErrorMotor> {
        let inicio = Instant::now();
        let url = format!("http://127.0.0.1:{}/health", self.puerto);
        loop {
            if let Some(estado) = self
                .proceso
                .try_wait()
                .contexto(|| "consultar el estado del motor".into())?
            {
                let causa = errores::causa_en_archivo(&self.registro);
                log::warn!(
                    "el motor se cerró al cargar (código {:?}, causa {causa:?})",
                    estado.code()
                );
                return Err(ErrorMotor::TerminoAlCargar {
                    codigo: estado.code(),
                    causa,
                    registro: self.registro.clone(),
                });
            }
            if cancelar.load(Ordering::Relaxed) {
                self.parar_proceso();
                return Err(ErrorMotor::Cancelado);
            }
            if inicio.elapsed() > espera_maxima {
                self.parar_proceso();
                return Err(ErrorMotor::TiempoAgotado {
                    segundos: espera_maxima.as_secs(),
                });
            }

            // /health does not need the key. 200 = ready; 503 = loading; connection error = not listening yet.
            let peticion = self
                .agente
                .get(&url)
                .config()
                .timeout_global(Some(Duration::from_secs(2)))
                .build();
            if let Ok(respuesta) = peticion.call()
                && respuesta.status().as_u16() == 200
            {
                return Ok(inicio.elapsed());
            }
            thread::sleep(INTERVALO_SALUD);
        }
    }

    /// Is the engine process still alive? (It can be asked with a shared engine: it does not need `&mut`.)
    pub fn sigue_vivo(&self) -> bool {
        locall_system::proceso_vivo(&self.proceso)
    }

    /// Why the engine exited, according to what it left written in its log (out of memory, damaged model…).
    pub fn causa_del_cierre(&self) -> Causa {
        errores::causa_en_archivo(&self.registro)
    }

    /// Local port the engine listens on.
    pub fn puerto(&self) -> u16 {
        self.puerto
    }

    /// Process identifier (to measure its memory with `locall-system`).
    pub fn pid(&self) -> u32 {
        self.proceso.id()
    }

    /// File with the engine's messages.
    pub fn registro(&self) -> &Path {
        &self.registro
    }

    /// Stops the engine and waits for it to finish. Same as dropping it, but reads better.
    pub fn parar(self) {
        drop(self);
    }

    fn parar_proceso(&mut self) {
        // On Windows, `kill` is an immediate stop (TerminateProcess). The engine saves nothing, so no information is lost.
        let _ = self.proceso.kill();
        let _ = self.proceso.wait();
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.puerto)
    }

    fn cabecera_clave(&self) -> String {
        format!("Bearer {}", self.clave)
    }
}

impl Drop for Motor {
    fn drop(&mut self) {
        self.parar_proceso();
        log::info!("motor parado (proceso {})", self.proceso.id());
    }
}

impl fmt::Debug for Motor {
    /// Without the key: it must never end up in a log or an error message.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Motor")
            .field("pid", &self.proceso.id())
            .field("puerto", &self.puerto)
            .field("registro", &self.registro)
            .finish_non_exhaustive()
    }
}

// ─── Preparation ──────────────────────────────────────────────────────────────

/// Random 32-byte key in hexadecimal (64 characters).
fn generar_clave() -> Result<String, ErrorMotor> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| ErrorMotor::Clave(e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Asks Windows for a free port (binding to port 0 makes the system choose one) and releases it for the engine.
///
/// There is a moment when another program could take it; then the engine does not start and it shows up as
/// [`ErrorMotor::TerminoAlCargar`] with [`Causa::PuertoOcupado`], and [`Motor::arrancar_listo`] retries with another.
fn puerto_libre() -> Result<u16, ErrorMotor> {
    let oyente = TcpListener::bind("127.0.0.1:0").contexto(|| "buscar un puerto libre".into())?;
    Ok(oyente
        .local_addr()
        .contexto(|| "leer el puerto libre".into())?
        .port())
}
