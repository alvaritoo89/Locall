//! # locall-engine
//!
//! Maneja el motor de inferencia, `llama-server` de llama.cpp (ADR 0003), como un **proceso aparte**:
//!
//! - Lo arranca con las reglas de seguridad de la Fase 0 (ADR 0006): solo `127.0.0.1`, un puerto nuevo en cada
//!   arranque, clave aleatoria pasada por **variable de entorno** (nunca como parámetro, que cualquier programa
//!   puede leer), CORS limitado a `localhost` y sin la web incorporada.
//! - Lo mete en un **Job Object** de Windows: si Locall muere, aunque sea de golpe, Windows mata el motor.
//! - Espera a que el modelo esté cargado y detecta si el motor muere durante la carga.
//! - Lo para siempre al soltarlo (`Drop`).
//!
//! Este crate no decide qué modelo cargar ni cuándo: eso es el planificador (regla 9).

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

mod chat;
#[cfg(test)]
mod pruebas;

pub use chat::{Evento, Fin, Mensaje, PeticionChat, Resumen, Rol};

/// Cada cuánto se pregunta al motor si ya está listo.
const INTERVALO_SALUD: Duration = Duration::from_millis(250);

// ─── Configuración ────────────────────────────────────────────────────────────

/// Con qué arrancar el motor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigMotor {
    /// Ruta de `llama-server.exe`. La carpeta elige el tipo de motor (CPU o Vulkan, D6).
    pub ejecutable: PathBuf,
    /// Ruta del modelo (en un modelo dividido, la primera parte).
    pub modelo: PathBuf,
    /// Tamaño del contexto en tokens.
    pub contexto: u32,
    /// Hilos del procesador (se validan contra los del equipo, D5).
    pub hilos: usize,
    /// Capas que se pasan a la gráfica. 0 = solo procesador.
    pub capas_gpu: u32,
    /// Archivo donde se guardan los mensajes del motor (se sobrescribe en cada arranque).
    pub registro: PathBuf,
}

/// Argumentos de `llama-server`. **No incluye la clave**: esa va por variable de entorno (D3).
pub fn argumentos(config: &ConfigMotor, puerto: u16) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();
    let mut par = |nombre: &str, valor: OsString| {
        args.push(nombre.into());
        args.push(valor);
    };
    par("--model", config.modelo.clone().into());
    par("--host", "127.0.0.1".into()); // D4: solo este equipo
    par("--port", puerto.to_string().into());
    par("--load-mode", "none".into()); // D2: sin mapeo de memoria
    par("--threads", config.hilos.to_string().into()); // D5
    par("--ctx-size", config.contexto.to_string().into());
    par("--n-gpu-layers", config.capas_gpu.to_string().into());
    par("--cors-origins", "localhost".into()); // D4: ninguna web ajena
    args.push("--no-webui".into());
    args
}

// ─── Errores ──────────────────────────────────────────────────────────────────

/// Lo que puede salir mal con el motor.
#[derive(Debug)]
pub enum ErrorMotor {
    /// No existe el ejecutable o el modelo.
    NoEncontrado(PathBuf),
    /// El número de hilos no es posible en este equipo.
    Hilos(HilosInvalidos),
    /// No se pudo generar la clave aleatoria.
    Clave(String),
    /// Error del sistema (lanzar el proceso, crear el registro, el Job Object…).
    Io { contexto: String, error: io::Error },
    /// El motor terminó antes de estar listo. Los detalles están en su registro.
    TerminoAlCargar {
        codigo: Option<i32>,
        registro: PathBuf,
    },
    /// El motor no estuvo listo en el tiempo máximo.
    TiempoAgotado { segundos: u64 },
    /// Se canceló la espera (el motor se ha parado).
    Cancelado,
    /// No se pudo hablar con el motor (conexión rechazada o cortada).
    Conexion(String),
    /// El motor respondió algo que no entendemos.
    Protocolo(String),
    /// El motor rechazó la petición. `tipo` y `mensaje` son los suyos (la parte 8c los traduce).
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
            ErrorMotor::Hilos(e) => write!(f, "{e}"),
            ErrorMotor::Clave(e) => write!(f, "no se pudo generar la clave del motor: {e}"),
            ErrorMotor::Io { contexto, error } => write!(f, "no se pudo {contexto}: {error}"),
            ErrorMotor::TerminoAlCargar { codigo, registro } => {
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
            } => write!(
                f,
                "el motor rechazó la petición ({estado}, {tipo}): {mensaje}"
            ),
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

/// Añade contexto a los errores de E/S (como en `locall-store`).
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

// ─── El motor ─────────────────────────────────────────────────────────────────

/// Un `llama-server` en marcha. Al soltarlo, se para.
pub struct Motor {
    proceso: Child,
    puerto: u16,
    clave: String,
    registro: PathBuf,
    agente: ureq::Agent,
    /// Debe vivir tanto como el proceso: al cerrarse, Windows mata el motor.
    _trabajo: Trabajo,
}

impl Motor {
    /// Lanza el motor. Vuelve enseguida: para esperar a que cargue el modelo, usa [`Motor::esperar_listo`].
    pub fn arrancar(config: &ConfigMotor) -> Result<Motor, ErrorMotor> {
        for ruta in [&config.ejecutable, &config.modelo] {
            if !ruta.is_file() {
                return Err(ErrorMotor::NoEncontrado(ruta.clone()));
            }
        }
        locall_system::validar_hilos(config.hilos).map_err(ErrorMotor::Hilos)?;

        let clave = generar_clave()?;
        let puerto = puerto_libre()?;

        // El trabajo se crea ANTES de lanzar el motor: si no se puede, no arrancamos.
        let trabajo = Trabajo::nuevo().contexto(|| "crear el Job Object".into())?;

        let registro = File::create(&config.registro)
            .contexto(|| format!("crear el registro {}", config.registro.display()))?;
        let registro_errores = registro
            .try_clone()
            .contexto(|| "duplicar el registro".into())?;

        let mut proceso = Command::new(&config.ejecutable)
            .args(argumentos(config, puerto))
            .env("LLAMA_API_KEY", &clave) // D3: la clave NUNCA como parámetro
            .stdin(Stdio::null())
            .stdout(Stdio::from(registro))
            .stderr(Stdio::from(registro_errores))
            .spawn()
            .contexto(|| format!("lanzar {}", config.ejecutable.display()))?;

        // Si no se puede meter en el trabajo, se para en vez de dejarlo suelto.
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
                .http_status_as_error(false) // un 503 ("cargando") es una respuesta normal
                .build(),
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

    /// Espera a que el modelo esté cargado. Devuelve cuánto ha tardado.
    ///
    /// Si el motor muere, se agota el tiempo o se cancela, el motor queda parado.
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
                return Err(ErrorMotor::TerminoAlCargar {
                    codigo: estado.code(),
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

            // /health no necesita la clave. 200 = listo; 503 = cargando; error de conexión = aún no escucha.
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

    /// ¿Sigue vivo el proceso del motor?
    pub fn sigue_vivo(&mut self) -> bool {
        matches!(self.proceso.try_wait(), Ok(None))
    }

    /// Puerto local donde escucha el motor.
    pub fn puerto(&self) -> u16 {
        self.puerto
    }

    /// Identificador del proceso (para medir su memoria con `locall-system`).
    pub fn pid(&self) -> u32 {
        self.proceso.id()
    }

    /// Archivo con los mensajes del motor.
    pub fn registro(&self) -> &Path {
        &self.registro
    }

    /// Para el motor y espera a que termine. Equivale a soltarlo, pero se lee mejor.
    pub fn parar(self) {
        drop(self);
    }

    fn parar_proceso(&mut self) {
        // En Windows, `kill` es una parada inmediata (TerminateProcess). El motor no guarda nada, así que no se
        // pierde información.
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
    }
}

impl fmt::Debug for Motor {
    /// Sin la clave: nunca debe acabar en un registro ni en un mensaje de error.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Motor")
            .field("pid", &self.proceso.id())
            .field("puerto", &self.puerto)
            .field("registro", &self.registro)
            .finish_non_exhaustive()
    }
}

// ─── Preparación ──────────────────────────────────────────────────────────────

/// Clave aleatoria de 32 bytes en hexadecimal (64 caracteres).
fn generar_clave() -> Result<String, ErrorMotor> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| ErrorMotor::Clave(e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Pide a Windows un puerto libre (abrir en el puerto 0 hace que el sistema elija uno) y lo libera para el motor.
///
/// Hay un instante en que otro programa podría ocuparlo; entonces el motor no arranca y se ve como
/// [`ErrorMotor::TerminoAlCargar`]. La parte 8c lo reconocerá en el registro para reintentar con otro puerto.
fn puerto_libre() -> Result<u16, ErrorMotor> {
    let oyente = TcpListener::bind("127.0.0.1:0").contexto(|| "buscar un puerto libre".into())?;
    Ok(oyente
        .local_addr()
        .contexto(|| "leer el puerto libre".into())?
        .port())
}
