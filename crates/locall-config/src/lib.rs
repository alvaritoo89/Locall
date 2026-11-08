//! # locall-config
//!
//! Dónde guarda Locall sus datos y los ajustes del usuario (`config.toml`).
//!
//! - **Carpeta de datos:** `%LOCALAPPDATA%\Locall` (en el equipo, no en la parte "Roaming" que Windows puede
//!   sincronizar: los modelos ocupan gigas). Con la variable `LOCALL_DATOS` se puede usar otra.
//! - **Ajustes:** un archivo de texto `clave = valor` (subconjunto de TOML, ver [`lector`]). Si no existe, se usan
//!   los valores por defecto. Cada valor se **valida** y los errores dicen la línea. Una clave desconocida es un
//!   [`Aviso`], no un error, para que un archivo de una versión más nueva no impida arrancar a una más antigua.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub mod lector;
pub mod registros;

use lector::{Entrada, Valor};

// ─── Carpetas ─────────────────────────────────────────────────────────────────

/// Las carpetas y archivos de Locall.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rutas {
    datos: PathBuf,
}

impl Rutas {
    /// `LOCALL_DATOS` si está definida; si no, `%LOCALAPPDATA%\Locall`.
    pub fn por_defecto() -> Result<Rutas, ErrorConfig> {
        let elegida = std::env::var_os("LOCALL_DATOS")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("LOCALAPPDATA")
                    .filter(|v| !v.is_empty())
                    .map(|base| PathBuf::from(base).join("Locall"))
            })
            .ok_or(ErrorConfig::SinCarpetaDeDatos)?;
        Ok(Rutas::en(elegida))
    }

    /// Usa `datos` como carpeta de datos.
    pub fn en(datos: impl Into<PathBuf>) -> Rutas {
        Rutas {
            datos: datos.into(),
        }
    }

    pub fn datos(&self) -> &Path {
        &self.datos
    }

    /// El archivo de ajustes.
    pub fn config(&self) -> PathBuf {
        self.datos.join("config.toml")
    }

    /// El registro de modelos instalados (SQLite).
    pub fn base_de_datos(&self) -> PathBuf {
        self.datos.join("locall.db")
    }

    /// El token de la API local (solo lo puede leer el usuario de Windows: está en su carpeta personal).
    pub fn token(&self) -> PathBuf {
        self.datos.join("token")
    }

    /// Carpeta de los registros (parte 10b).
    pub fn registros(&self) -> PathBuf {
        self.datos.join("registros")
    }

    /// Carpeta del almacén de modelos si el usuario no elige otra.
    pub fn modelos_por_defecto(&self) -> PathBuf {
        self.datos.join("modelos")
    }
}

// ─── Ajustes ──────────────────────────────────────────────────────────────────

/// Hilos del procesador para el motor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hilos {
    /// Todos los del equipo (D5).
    Auto,
    /// Un número concreto (ya validado contra los del equipo).
    Numero(usize),
}

/// Los ajustes del usuario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Carpeta del almacén de modelos. `None` = la de por defecto ([`Rutas::modelos_por_defecto`]).
    pub carpeta_modelos: Option<PathBuf>,
    pub hilos: Hilos,
    /// Contexto por defecto, en tokens.
    pub contexto: u32,
    /// Minutos sin uso antes de descargar el modelo (0 = nunca).
    pub inactividad_minutos: u32,
    /// Conversaciones a la vez por modelo (ADR 0012).
    pub paralelo: u32,
    /// Días que se guardan los registros.
    pub registros_dias: u32,
    /// Puerto de la API local (solo en 127.0.0.1).
    pub puerto: u16,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            carpeta_modelos: None,
            hilos: Hilos::Auto,
            contexto: 4096,
            inactividad_minutos: 5,
            paralelo: 1,
            registros_dias: 7,
            puerto: PUERTO_POR_DEFECTO,
        }
    }
}

/// El archivo que se crea la primera vez: los valores por defecto, explicados.
/// (Una prueba comprueba que, al leerlo, da exactamente [`Config::default`].)
pub const ARCHIVO_POR_DEFECTO: &str = r#"# Ajustes de Locall.
# Si borras una línea, se usa el valor por defecto. Las rutas, entre comillas simples: 'D:\Modelos'

# Carpeta de los modelos (vacía = la carpeta "modelos" junto a este archivo)
carpeta_modelos = ''

# Hilos del procesador para el motor: "auto" (todos) o un número
hilos = "auto"

# Contexto por defecto, en tokens (cuánto "recuerda" el modelo de la conversación)
contexto = 4096

# Minutos sin usarse antes de descargar el modelo de la memoria (0 = nunca)
inactividad_minutos = 5

# Conversaciones que un modelo atiende a la vez
paralelo = 1

# Días que se guardan los registros
registros_dias = 7

# Puerto de la API local (solo accesible desde este equipo)
puerto = 11470
"#;

/// Puerto de la API por defecto. Elegido para no coincidir con los de Ollama (11434), LM Studio (1234) ni Jan (1337).
pub const PUERTO_POR_DEFECTO: u16 = 11470;

/// Límites de cada ajuste numérico: (mínimo, máximo).
const LIMITES_CONTEXTO: (i64, i64) = (256, 1_048_576);
const LIMITES_INACTIVIDAD: (i64, i64) = (0, 7 * 24 * 60); // hasta una semana
const LIMITES_PARALELO: (i64, i64) = (1, 16);
const LIMITES_REGISTROS: (i64, i64) = (1, 365);
const LIMITES_PUERTO: (i64, i64) = (1024, 65535); // por debajo de 1024 hacen falta permisos de administrador

impl Config {
    /// Lee los ajustes de un texto. Devuelve también los avisos (claves desconocidas).
    pub fn desde_texto(texto: &str) -> Result<(Config, Vec<Aviso>), ErrorConfig> {
        let entradas = lector::leer(texto).map_err(|e| ErrorConfig::Sintaxis {
            linea: e.linea,
            mensaje: e.mensaje,
        })?;
        let mut config = Config::default();
        let mut avisos = Vec::new();
        for entrada in &entradas {
            match entrada.clave.as_str() {
                "carpeta_modelos" => config.carpeta_modelos = carpeta(entrada)?,
                "hilos" => config.hilos = hilos(entrada)?,
                "contexto" => config.contexto = numero(entrada, LIMITES_CONTEXTO)?,
                "inactividad_minutos" => {
                    config.inactividad_minutos = numero(entrada, LIMITES_INACTIVIDAD)?;
                }
                "paralelo" => config.paralelo = numero(entrada, LIMITES_PARALELO)?,
                "registros_dias" => config.registros_dias = numero(entrada, LIMITES_REGISTROS)?,
                "puerto" => {
                    config.puerto = u16::try_from(numero(entrada, LIMITES_PUERTO)?)
                        .map_err(|_| error_valor(entrada, "fuera de rango"))?;
                }
                _ => avisos.push(Aviso::ClaveDesconocida {
                    linea: entrada.linea,
                    clave: entrada.clave.clone(),
                }),
            }
        }
        Ok((config, avisos))
    }

    /// Lee el archivo de ajustes. Si no existe, devuelve los valores por defecto (sin crearlo).
    pub fn leer(ruta: &Path) -> Result<(Config, Vec<Aviso>), ErrorConfig> {
        match std::fs::read_to_string(ruta) {
            Ok(texto) => Config::desde_texto(&texto),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok((Config::default(), Vec::new())),
            Err(e) if e.kind() == io::ErrorKind::InvalidData => Err(ErrorConfig::Sintaxis {
                linea: 0,
                mensaje: "el archivo no es texto UTF-8; guárdalo como UTF-8".into(),
            }),
            Err(error) => Err(ErrorConfig::Io {
                ruta: ruta.to_path_buf(),
                error,
            }),
        }
    }

    /// Crea el archivo de ajustes con los valores por defecto **si no existe** (nunca sobrescribe el del usuario).
    /// Devuelve si lo ha creado.
    pub fn crear_si_no_existe(rutas: &Rutas) -> Result<bool, ErrorConfig> {
        let ruta = rutas.config();
        let io = |error| ErrorConfig::Io {
            ruta: ruta.clone(),
            error,
        };
        std::fs::create_dir_all(rutas.datos()).map_err(io)?;
        // `create_new` falla si ya existe: así no hay forma de pisar el archivo del usuario.
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&ruta)
        {
            Ok(mut archivo) => {
                use std::io::Write;
                archivo
                    .write_all(ARCHIVO_POR_DEFECTO.as_bytes())
                    .map_err(io)?;
                Ok(true)
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(false),
            Err(e) => Err(io(e)),
        }
    }

    /// Hilos que se darán al motor.
    pub fn hilos_para_motor(&self) -> usize {
        match self.hilos {
            Hilos::Auto => locall_system::hilos_para_motor(),
            Hilos::Numero(n) => n,
        }
    }

    /// Carpeta del almacén de modelos.
    pub fn carpeta_modelos(&self, rutas: &Rutas) -> PathBuf {
        self.carpeta_modelos
            .clone()
            .unwrap_or_else(|| rutas.modelos_por_defecto())
    }

    /// Tiempo de inactividad para el planificador (`None` = no descargar nunca).
    pub fn inactividad(&self) -> Option<Duration> {
        (self.inactividad_minutos > 0)
            .then(|| Duration::from_secs(u64::from(self.inactividad_minutos) * 60))
    }
}

// ─── Validación de cada ajuste ───────────────────────────────────────────────

fn error_valor(entrada: &Entrada, mensaje: impl Into<String>) -> ErrorConfig {
    ErrorConfig::Valor {
        linea: entrada.linea,
        clave: entrada.clave.clone(),
        mensaje: mensaje.into(),
    }
}

fn numero(entrada: &Entrada, (minimo, maximo): (i64, i64)) -> Result<u32, ErrorConfig> {
    match entrada.valor {
        Valor::Entero(n) if (minimo..=maximo).contains(&n) => {
            u32::try_from(n).map_err(|_| error_valor(entrada, "fuera de rango"))
        }
        Valor::Entero(n) => Err(error_valor(
            entrada,
            format!("{n} no es válido: debe estar entre {minimo} y {maximo}"),
        )),
        Valor::Texto(_) => Err(error_valor(entrada, "debe ser un número, sin comillas")),
    }
}

fn hilos(entrada: &Entrada) -> Result<Hilos, ErrorConfig> {
    match &entrada.valor {
        Valor::Texto(t) if t.eq_ignore_ascii_case("auto") => Ok(Hilos::Auto),
        Valor::Texto(t) => Err(error_valor(
            entrada,
            format!("«{t}» no es válido: escribe \"auto\" o un número"),
        )),
        Valor::Entero(n) => {
            let n = usize::try_from(*n)
                .map_err(|_| error_valor(entrada, format!("{n} no es válido")))?;
            locall_system::validar_hilos(n)
                .map(Hilos::Numero)
                .map_err(|e| error_valor(entrada, e.to_string()))
        }
    }
}

fn carpeta(entrada: &Entrada) -> Result<Option<PathBuf>, ErrorConfig> {
    match &entrada.valor {
        Valor::Texto(t) if t.trim().is_empty() => Ok(None),
        Valor::Texto(t) => {
            let ruta = PathBuf::from(t.trim());
            if ruta.is_absolute() {
                Ok(Some(ruta))
            } else {
                Err(error_valor(
                    entrada,
                    format!("«{t}» debe ser una ruta completa, por ejemplo 'D:\\Modelos'"),
                ))
            }
        }
        Valor::Entero(_) => Err(error_valor(
            entrada,
            "debe ser una ruta entre comillas simples, por ejemplo 'D:\\Modelos'",
        )),
    }
}

// ─── Errores y avisos ─────────────────────────────────────────────────────────

/// Algo que no impide arrancar pero conviene decir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Aviso {
    /// Un ajuste que esta versión no conoce (quizá de una versión más nueva, o una errata). Se ignora.
    ClaveDesconocida { linea: usize, clave: String },
}

impl fmt::Display for Aviso {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Aviso::ClaveDesconocida { linea, clave } => write!(
                f,
                "línea {linea}: el ajuste «{clave}» no existe en esta versión de Locall; se ignora"
            ),
        }
    }
}

/// Lo que puede salir mal con la configuración.
#[derive(Debug)]
pub enum ErrorConfig {
    /// No se sabe dónde guardar los datos (sin `LOCALL_DATOS` ni `LOCALAPPDATA`).
    SinCarpetaDeDatos,
    /// El archivo está mal escrito.
    Sintaxis { linea: usize, mensaje: String },
    /// Un ajuste tiene un valor no válido.
    Valor {
        linea: usize,
        clave: String,
        mensaje: String,
    },
    /// No se pudo leer o escribir el archivo.
    Io { ruta: PathBuf, error: io::Error },
}

impl fmt::Display for ErrorConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorConfig::SinCarpetaDeDatos => write!(
                f,
                "no se encuentra la carpeta de datos del usuario (falta la variable LOCALAPPDATA)"
            ),
            ErrorConfig::Sintaxis { linea: 0, mensaje } => {
                write!(f, "config.toml: {mensaje}")
            }
            ErrorConfig::Sintaxis { linea, mensaje } => {
                write!(f, "config.toml, línea {linea}: {mensaje}")
            }
            ErrorConfig::Valor {
                linea,
                clave,
                mensaje,
            } => write!(f, "config.toml, línea {linea} («{clave}»): {mensaje}"),
            ErrorConfig::Io { ruta, error } => {
                write!(f, "no se pudo usar {}: {error}", ruta.display())
            }
        }
    }
}

impl std::error::Error for ErrorConfig {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ErrorConfig::Io { error, .. } => Some(error),
            _ => None,
        }
    }
}
