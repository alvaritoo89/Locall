//! # locall-config
//!
//! Where Locall keeps its data and the user's settings (`config.toml`).
//!
//! - **Data folder:** `%LOCALAPPDATA%\Locall` (on the computer, not in the "Roaming" part Windows may sync: models
//!   take gigabytes). Another one can be used with the `LOCALL_DATOS` variable.
//! - **Settings:** a `key = value` text file (a subset of TOML, see [`lector`]). If it does not exist, the defaults
//!   are used. Every value is **validated** and errors give the line. An unknown key is an [`Aviso`], not an error,
//!   so a file from a newer version does not stop an older one from starting.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub mod lector;
pub mod registros;

use lector::{Entrada, Valor};

// ─── Folders ──────────────────────────────────────────────────────────────────

/// Locall's folders and files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rutas {
    datos: PathBuf,
}

impl Rutas {
    /// `LOCALL_DATOS` if it is set; otherwise `%LOCALAPPDATA%\Locall`.
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

    /// Uses `datos` as the data folder.
    pub fn en(datos: impl Into<PathBuf>) -> Rutas {
        Rutas {
            datos: datos.into(),
        }
    }

    pub fn datos(&self) -> &Path {
        &self.datos
    }

    /// The settings file.
    pub fn config(&self) -> PathBuf {
        self.datos.join("config.toml")
    }

    /// The registry of installed models (SQLite).
    pub fn base_de_datos(&self) -> PathBuf {
        self.datos.join("locall.db")
    }

    /// The local API token (only the Windows user can read it: it is in their personal folder).
    pub fn token(&self) -> PathBuf {
        self.datos.join("token")
    }

    /// Logs folder (part 10b).
    pub fn registros(&self) -> PathBuf {
        self.datos.join("registros")
    }

    /// Folder of the downloaded model catalog (with its signature).
    pub fn catalogo(&self) -> PathBuf {
        self.datos.join("catalogo")
    }

    /// Model store folder if the user does not choose another one.
    pub fn modelos_por_defecto(&self) -> PathBuf {
        self.datos.join("modelos")
    }
}

// ─── Settings ─────────────────────────────────────────────────────────────────

/// Processor threads for the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hilos {
    /// All of the computer's (D5).
    Auto,
    /// A specific number (already validated against the computer's).
    Numero(usize),
}

/// The user's settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Model store folder. `None` = the default one ([`Rutas::modelos_por_defecto`]).
    pub carpeta_modelos: Option<PathBuf>,
    pub hilos: Hilos,
    /// Default context, in tokens.
    pub contexto: u32,
    /// Idle minutes before unloading the model (0 = never).
    pub inactividad_minutos: u32,
    /// Simultaneous conversations per model (ADR 0012).
    pub paralelo: u32,
    /// Days the logs are kept.
    pub registros_dias: u32,
    /// Local API port (only on 127.0.0.1).
    pub puerto: u16,
    /// Idle minutes after which Locall closes by itself (0 = never). The CLI and the app open it again when needed
    /// (ADR 0015).
    pub cierre_inactividad_minutos: u32,
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
            cierre_inactividad_minutos: 30,
        }
    }
}

/// The file created the first time: the defaults, explained.
/// (A test checks that reading it gives exactly [`Config::default`].)
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

# Minutos sin usarse tras los que Locall se cierra solo (0 = nunca)
cierre_inactividad_minutos = 30
"#;

/// Default API port. Chosen not to clash with Ollama's (11434), LM Studio's (1234) or Jan's (1337).
pub const PUERTO_POR_DEFECTO: u16 = 11470;

/// Limits of each numeric setting: (minimum, maximum).
const LIMITES_CONTEXTO: (i64, i64) = (256, 1_048_576);
const LIMITES_INACTIVIDAD: (i64, i64) = (0, 7 * 24 * 60); // up to one week
const LIMITES_PARALELO: (i64, i64) = (1, 16);
const LIMITES_REGISTROS: (i64, i64) = (1, 365);
const LIMITES_PUERTO: (i64, i64) = (1024, 65535); // below 1024 administrator permissions are needed

impl Config {
    /// Reads the settings from a text. Also returns the warnings (unknown keys).
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
                "cierre_inactividad_minutos" => {
                    config.cierre_inactividad_minutos = numero(entrada, LIMITES_INACTIVIDAD)?;
                }
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

    /// Reads the settings file. If it does not exist, returns the defaults (without creating it).
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

    /// Creates the settings file with the defaults **if it does not exist** (never overwrites the user's).
    /// Returns whether it was created.
    pub fn crear_si_no_existe(rutas: &Rutas) -> Result<bool, ErrorConfig> {
        let ruta = rutas.config();
        let io = |error| ErrorConfig::Io {
            ruta: ruta.clone(),
            error,
        };
        std::fs::create_dir_all(rutas.datos()).map_err(io)?;
        // `create_new` fails if it already exists: this way there is no way to overwrite the user's file.
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

    /// Saves these changes to the settings file (creates it if it does not exist) and returns the saved settings.
    ///
    /// - Only the settings in [`EDITABLES`] can be changed.
    /// - **Only the lines** of those settings are rewritten (if a setting is not in the file, it is appended): comments
    ///   and the rest of the file stay as they were.
    /// - The result is **validated before writing it**, and it is written to a temporary file that then replaces the
    ///   original: an interruption halfway never leaves a broken `config.toml`.
    /// - Changes take effect the next time Locall is opened.
    pub fn guardar_cambios(
        rutas: &Rutas,
        cambios: &[(&str, Valor)],
    ) -> Result<Config, ErrorConfig> {
        let ruta = rutas.config();
        let actual = match std::fs::read_to_string(&ruta) {
            Ok(texto) => texto,
            Err(e) if e.kind() == io::ErrorKind::NotFound => ARCHIVO_POR_DEFECTO.to_owned(),
            Err(error) => return Err(ErrorConfig::Io { ruta, error }),
        };
        let nuevo = texto_con_cambios(&actual, cambios)?;
        let (config, _) = Config::desde_texto(&nuevo)?;
        let temporal = ruta.with_extension("toml.nuevo");
        let io = |error| ErrorConfig::Io {
            ruta: ruta.clone(),
            error,
        };
        std::fs::create_dir_all(rutas.datos()).map_err(io)?;
        std::fs::write(&temporal, nuevo.as_bytes()).map_err(io)?;
        // On Windows, `rename` replaces the target file if it already exists (MoveFileEx with REPLACE_EXISTING).
        if let Err(e) = std::fs::rename(&temporal, &ruta) {
            let _ = std::fs::remove_file(&temporal);
            return Err(io(e));
        }
        Ok(config)
    }

    /// Threads that will be given to the engine.
    pub fn hilos_para_motor(&self) -> usize {
        match self.hilos {
            Hilos::Auto => locall_system::hilos_para_motor(),
            Hilos::Numero(n) => n,
        }
    }

    /// Model store folder.
    pub fn carpeta_modelos(&self, rutas: &Rutas) -> PathBuf {
        self.carpeta_modelos
            .clone()
            .unwrap_or_else(|| rutas.modelos_por_defecto())
    }

    /// Idle time after which Locall closes by itself (`None` = never).
    pub fn cierre_inactividad(&self) -> Option<Duration> {
        (self.cierre_inactividad_minutos > 0)
            .then(|| Duration::from_secs(u64::from(self.cierre_inactividad_minutos) * 60))
    }

    /// Idle time for the scheduler (`None` = never unload).
    pub fn inactividad(&self) -> Option<Duration> {
        (self.inactividad_minutos > 0)
            .then(|| Duration::from_secs(u64::from(self.inactividad_minutos) * 60))
    }
}

// ─── Changing settings ────────────────────────────────────────────────────────

/// The settings that can be changed from the app. The others (models folder, port, parallel) only by hand in
/// `config.toml`: changing them with Locall open has consequences the app does not handle yet (ADR 0020).
pub const EDITABLES: [&str; 5] = [
    "contexto",
    "hilos",
    "inactividad_minutos",
    "cierre_inactividad_minutos",
    "registros_dias",
];

/// The file text with these changes (without validating the values: [`Config::desde_texto`] does that).
pub fn texto_con_cambios(texto: &str, cambios: &[(&str, Valor)]) -> Result<String, ErrorConfig> {
    let entradas = lector::leer(texto).map_err(|e| ErrorConfig::Sintaxis {
        linea: e.linea,
        mensaje: e.mensaje,
    })?;
    let salto = if texto.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lineas: Vec<String> = texto.lines().map(str::to_owned).collect();
    let mut nuevas = Vec::new();
    for (clave, valor) in cambios {
        if !EDITABLES.contains(clave) {
            return Err(ErrorConfig::NoEditable((*clave).to_owned()));
        }
        let linea = format!("{clave} = {}", escribir(valor));
        match entradas.iter().find(|e| e.clave == *clave) {
            Some(e) => lineas[e.linea - 1] = linea,
            None => nuevas.push(linea),
        }
    }
    lineas.extend(nuevas);
    let mut resultado = lineas.join(salto);
    resultado.push_str(salto);
    Ok(resultado)
}

/// A value as it is written in the file. Texts go in double quotes, with `\` and `"` escaped.
fn escribir(valor: &Valor) -> String {
    match valor {
        Valor::Entero(n) => n.to_string(),
        Valor::Texto(t) => format!("\"{}\"", t.replace('\\', "\\\\").replace('"', "\\\"")),
    }
}

// ─── Validation of each setting ───────────────────────────────────────────────

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

// ─── Errors and warnings ──────────────────────────────────────────────────────

/// Something that does not prevent starting but is worth reporting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Aviso {
    /// A setting this version does not know (maybe from a newer version, or a typo). It is ignored.
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

/// What can go wrong with the configuration.
#[derive(Debug)]
pub enum ErrorConfig {
    /// It is unknown where to keep the data (no `LOCALL_DATOS` nor `LOCALAPPDATA`).
    SinCarpetaDeDatos,
    /// The file is malformed.
    Sintaxis { linea: usize, mensaje: String },
    /// A setting has an invalid value.
    Valor {
        linea: usize,
        clave: String,
        mensaje: String,
    },
    /// The file could not be read or written.
    Io { ruta: PathBuf, error: io::Error },
    /// An attempt was made to change from the app a setting that is not in [`EDITABLES`].
    NoEditable(String),
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
            ErrorConfig::NoEditable(clave) => write!(
                f,
                "el ajuste «{clave}» no se puede cambiar desde aquí; cámbialo en config.toml"
            ),
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
