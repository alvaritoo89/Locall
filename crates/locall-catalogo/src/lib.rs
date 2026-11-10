//! # locall-catalogo
//!
//! El **catálogo de modelos compatibles** de Locall (ADR 0022): qué modelos recomienda Locall, de dónde se descargan
//! exactamente (repositorio, commit fijado, archivos con su sha256) y para qué sirven.
//!
//! - El catálogo es un JSON (`catalogo/catalogo.json` en el repositorio) **firmado con minisign** por quien publica
//!   Locall (`catalogo.json.minisig`). La app y la CLI llevan dentro la clave pública ([`CLAVE_PUBLICA`]).
//! - [`verificar`] comprueba **primero la firma** y solo después lee el contenido: nunca se interpreta un archivo
//!   sin firmar. Después lo valida campo por campo, con límites de tamaño.
//! - Estar en el catálogo significa **compatible**: se leyó la cabecera del archivo exacto (ese commit) y el motor que
//!   indica `checked_engine` reconoce su arquitectura y sus tipos de datos. **No** significa que se haya probado a
//!   conversar con él: la app y la web lo dicen así.
//!
//! ```json
//! {
//!   "format": 1,
//!   "published": 1794300000,
//!   "models": [{
//!     "name": "qwen3-4b", "title": "Qwen3 4B", "publisher": "Qwen",
//!     "description": "…", "tags": ["conversar"], "license": "apache-2.0", "context_length": 40960,
//!     "variants": [{
//!       "name": "q4_k_m", "default": true, "repo": "Qwen/Qwen3-4B-GGUF", "commit": "<40 hex>",
//!       "checked_engine": "b12189",
//!       "files": [{"path": "Qwen3-4B-Q4_K_M.gguf", "size": 2497280256, "sha256": "<64 hex>"}]
//!     }]
//!   }]
//! }
//! ```
//!
//! Los campos que esta versión no conoce se ignoran (para que un catálogo nuevo con campos de más siga sirviendo),
//! pero un `format` mayor que [`FORMATO`] es un error: ese catálogo necesita un Locall más nuevo.

use std::collections::BTreeSet;
use std::fmt;

use minisign_verify::{PublicKey, Signature};
use serde_json::{Map, Value};

/// La versión del formato que entiende este Locall.
pub const FORMATO: u64 = 1;

/// La clave pública con la que se firma el catálogo oficial (minisign, id `5C09624CD1482FE0`).
pub const CLAVE_PUBLICA: &str = "RWTgL0jRTGIJXDFqcD3bREOcjTX2CAHHzp1u+orTwHN5XscqH2mHvk3S";

/// Tamaño máximo del catálogo: si alguien sirve un archivo enorme, se rechaza sin leerlo.
pub const TAMANO_MAX: usize = 4 * 1024 * 1024;

const MODELOS_MAX: usize = 500;
const VARIANTES_MAX: usize = 16;
const ARCHIVOS_MAX: usize = 64;
const ETIQUETAS_MAX: usize = 12;
const NOMBRE_MAX: usize = 48;
const TEXTO_CORTO_MAX: usize = 120;
const DESCRIPCION_MAX: usize = 2000;

/// El catálogo, ya verificado y validado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalogo {
    /// Cuándo se publicó (segundos desde 1970). Sirve para no aceptar nunca uno más antiguo que el último aceptado.
    pub publicado: u64,
    pub modelos: Vec<ModeloCatalogo>,
}

/// Un modelo del catálogo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeloCatalogo {
    /// Nombre corto, el que se escribe en `locall pull`: `qwen3-4b`.
    pub nombre: String,
    /// Nombre para mostrar: `Qwen3 4B`.
    pub titulo: String,
    /// Quién lo publica: `Qwen`.
    pub autor: String,
    /// Para qué sirve, en español.
    pub descripcion: String,
    /// Etiquetas para los filtros (`conversar`, `programar`…).
    pub etiquetas: Vec<String>,
    /// Licencia (`apache-2.0`, `mit`…).
    pub licencia: String,
    /// Contexto máximo que admite el modelo, en tokens.
    pub contexto_max: u64,
    /// Las variantes (cuantizaciones). Exactamente una es la de por defecto.
    pub variantes: Vec<Variante>,
}

/// Una variante de un modelo: un archivo GGUF concreto (o varios, si está dividido).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variante {
    /// `q4_k_m`, `q8_0`…
    pub nombre: String,
    pub por_defecto: bool,
    /// Repositorio de Hugging Face: `Qwen/Qwen3-4B-GGUF`.
    pub repo: String,
    /// Commit exacto (40 caracteres hexadecimales, en minúsculas).
    pub commit: String,
    /// Versión del motor con la que se comprobó la compatibilidad (`b12189`).
    pub motor_comprobado: String,
    pub archivos: Vec<ArchivoCatalogo>,
}

/// Un archivo de una variante.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivoCatalogo {
    /// Ruta dentro del repositorio (puede tener carpetas).
    pub ruta: String,
    pub bytes: u64,
    /// sha256 en hexadecimal (64 caracteres, minúsculas).
    pub sha256: String,
}

impl Variante {
    /// Lo que ocupa en el disco.
    pub fn bytes(&self) -> u64 {
        self.archivos.iter().map(|a| a.bytes).sum()
    }
}

impl ModeloCatalogo {
    /// La variante de por defecto (siempre hay una: lo garantiza la validación).
    pub fn por_defecto(&self) -> &Variante {
        self.variantes
            .iter()
            .find(|v| v.por_defecto)
            .unwrap_or(&self.variantes[0])
    }
}

impl Catalogo {
    /// Busca por lo que escribe la persona: `qwen3-4b` (la variante de por defecto) o `qwen3-4b:q8_0`. No distingue
    /// mayúsculas.
    pub fn resolver(&self, texto: &str) -> Option<(&ModeloCatalogo, &Variante)> {
        let texto = texto.trim().to_ascii_lowercase();
        let (nombre, variante) = match texto.split_once(':') {
            Some((n, v)) => (n, Some(v)),
            None => (texto.as_str(), None),
        };
        let modelo = self.modelos.iter().find(|m| m.nombre == nombre)?;
        let variante = match variante {
            None => modelo.por_defecto(),
            Some(v) => modelo.variantes.iter().find(|x| x.nombre == v)?,
        };
        Some((modelo, variante))
    }
}

// ─── El catálogo que va dentro de Locall ──────────────────────────────────────

/// El catálogo oficial tal como estaba al compilar este Locall (`catalogo/catalogo.json`), con su firma. Sirve sin
/// internet y como punto de partida: la app lo sustituye por uno más nuevo si lo descarga y su firma es buena.
pub const CATALOGO_INCLUIDO: &[u8] = include_bytes!("../../../catalogo/catalogo.json");
/// La firma de [`CATALOGO_INCLUIDO`].
pub const FIRMA_INCLUIDA: &str = include_str!("../../../catalogo/catalogo.json.minisig");

/// El catálogo que va dentro de Locall, verificado igual que uno descargado (con [`CLAVE_PUBLICA`]).
pub fn incluido() -> Result<Catalogo, ErrorCatalogo> {
    verificar(CATALOGO_INCLUIDO, FIRMA_INCLUIDA, CLAVE_PUBLICA)
}

// ─── Verificar ────────────────────────────────────────────────────────────────

/// Comprueba la firma de `datos` con `clave_publica` (base64 de minisign) y, si es buena, lee y valida el catálogo.
///
/// Solo se aceptan firmas del formato actual de minisign (con el archivo resumido con BLAKE2b, el que usa
/// minisign 0.12 por defecto); las del formato antiguo se rechazan.
pub fn verificar(
    datos: &[u8],
    firma: &str,
    clave_publica: &str,
) -> Result<Catalogo, ErrorCatalogo> {
    if datos.len() > TAMANO_MAX {
        return Err(ErrorCatalogo::DemasiadoGrande(datos.len()));
    }
    let clave = PublicKey::from_base64(clave_publica).map_err(|_| ErrorCatalogo::ClaveNoValida)?;
    let firma = Signature::decode(firma).map_err(|_| ErrorCatalogo::FirmaIlegible)?;
    clave
        .verify(datos, &firma, false)
        .map_err(|_| ErrorCatalogo::FirmaIncorrecta)?;
    // A partir de aquí, el contenido es de quien tiene la clave privada.
    leer(datos)
}

/// Lee y valida un catálogo **sin comprobar la firma**. Solo para quien publica (preparar y revisar un catálogo antes
/// de firmarlo) y para las pruebas; la app usa siempre [`verificar`].
pub fn leer(datos: &[u8]) -> Result<Catalogo, ErrorCatalogo> {
    if datos.len() > TAMANO_MAX {
        return Err(ErrorCatalogo::DemasiadoGrande(datos.len()));
    }
    let json: Value =
        serde_json::from_slice(datos).map_err(|e| ErrorCatalogo::Json(e.to_string()))?;
    let raiz = objeto(&json, "el catálogo")?;
    let formato = numero(raiz, "format", "el catálogo")?;
    if formato > FORMATO {
        return Err(ErrorCatalogo::FormatoNuevo(formato));
    }
    if formato != FORMATO {
        return Err(invalido("el catálogo", "«format» no es válido"));
    }
    let publicado = numero(raiz, "published", "el catálogo")?;
    if publicado == 0 {
        return Err(invalido("el catálogo", "«published» debe ser una fecha"));
    }
    let lista = lista(raiz, "models", "el catálogo", MODELOS_MAX)?;
    let mut modelos = Vec::with_capacity(lista.len());
    let mut nombres = BTreeSet::new();
    for (i, m) in lista.iter().enumerate() {
        let modelo = leer_modelo(m, i)?;
        if !nombres.insert(modelo.nombre.clone()) {
            return Err(invalido(
                &format!("el modelo «{}»", modelo.nombre),
                "está repetido",
            ));
        }
        modelos.push(modelo);
    }
    Ok(Catalogo { publicado, modelos })
}

fn leer_modelo(valor: &Value, i: usize) -> Result<ModeloCatalogo, ErrorCatalogo> {
    let donde = format!("el modelo {}", i + 1);
    let m = objeto(valor, &donde)?;
    let nombre = texto(m, "name", &donde, NOMBRE_MAX)?;
    if !nombre_valido(&nombre) {
        return Err(invalido(
            &donde,
            "«name» solo puede tener minúsculas, números, «-» y «.» (y empezar por letra o número)",
        ));
    }
    let donde = format!("el modelo «{nombre}»");
    let etiquetas: Vec<String> = lista(m, "tags", &donde, ETIQUETAS_MAX)?
        .iter()
        .map(|e| {
            e.as_str()
                .filter(|t| !t.is_empty() && t.len() <= 32 && nombre_valido(t))
                .map(str::to_owned)
                .ok_or_else(|| invalido(&donde, "cada etiqueta debe ser una palabra en minúsculas"))
        })
        .collect::<Result<_, _>>()?;
    let lista_variantes = lista(m, "variants", &donde, VARIANTES_MAX)?;
    if lista_variantes.is_empty() {
        return Err(invalido(&donde, "no tiene ninguna variante"));
    }
    let mut variantes = Vec::with_capacity(lista_variantes.len());
    let mut nombres = BTreeSet::new();
    for v in lista_variantes {
        let variante = leer_variante(v, &donde)?;
        if !nombres.insert(variante.nombre.clone()) {
            return Err(invalido(
                &donde,
                &format!("la variante «{}» está repetida", variante.nombre),
            ));
        }
        variantes.push(variante);
    }
    if variantes.iter().filter(|v| v.por_defecto).count() != 1 {
        return Err(invalido(
            &donde,
            "debe tener exactamente una variante con «default»: true",
        ));
    }
    Ok(ModeloCatalogo {
        titulo: texto(m, "title", &donde, TEXTO_CORTO_MAX)?,
        autor: texto(m, "publisher", &donde, TEXTO_CORTO_MAX)?,
        descripcion: texto(m, "description", &donde, DESCRIPCION_MAX)?,
        licencia: texto(m, "license", &donde, 64)?,
        contexto_max: numero(m, "context_length", &donde)?,
        nombre,
        etiquetas,
        variantes,
    })
}

fn leer_variante(valor: &Value, modelo: &str) -> Result<Variante, ErrorCatalogo> {
    let v = objeto(valor, modelo)?;
    let nombre = texto(v, "name", modelo, 32)?;
    if !nombre
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(invalido(
            modelo,
            &format!(
                "la variante «{nombre}»: el nombre solo puede tener minúsculas, números y «_»"
            ),
        ));
    }
    let donde = format!("{modelo}, variante «{nombre}»");
    let por_defecto = match v.get("default") {
        None | Some(Value::Bool(false)) => false,
        Some(Value::Bool(true)) => true,
        Some(_) => return Err(invalido(&donde, "«default» debe ser true o false")),
    };
    let repo = texto(v, "repo", &donde, 200)?;
    if !repo_valido(&repo) {
        return Err(invalido(&donde, "«repo» debe ser propietario/nombre"));
    }
    let commit = texto(v, "commit", &donde, 40)?;
    if !hexadecimal(&commit, 40) {
        return Err(invalido(
            &donde,
            "«commit» debe ser un commit completo (40 caracteres hexadecimales, en minúsculas)",
        ));
    }
    let motor_comprobado = texto(v, "checked_engine", &donde, 32)?;
    let lista_archivos = lista(v, "files", &donde, ARCHIVOS_MAX)?;
    if lista_archivos.is_empty() {
        return Err(invalido(&donde, "no tiene archivos"));
    }
    let mut archivos = Vec::with_capacity(lista_archivos.len());
    for a in lista_archivos {
        let a = objeto(a, &donde)?;
        let ruta = texto(a, "path", &donde, 300)?;
        if !ruta_valida(&ruta) {
            return Err(invalido(
                &donde,
                &format!("«{ruta}» no es una ruta válida de un archivo .gguf"),
            ));
        }
        let bytes = numero(a, "size", &donde)?;
        if bytes == 0 {
            return Err(invalido(&donde, &format!("«{ruta}» tiene tamaño 0")));
        }
        let sha256 = texto(a, "sha256", &donde, 64)?;
        if !hexadecimal(&sha256, 64) {
            return Err(invalido(
                &donde,
                &format!("«{ruta}»: «sha256» debe tener 64 caracteres hexadecimales en minúsculas"),
            ));
        }
        archivos.push(ArchivoCatalogo {
            ruta,
            bytes,
            sha256,
        });
    }
    Ok(Variante {
        nombre,
        por_defecto,
        repo,
        commit,
        motor_comprobado,
        archivos,
    })
}

// ─── Reglas de cada campo ─────────────────────────────────────────────────────

/// Nombre corto: minúsculas, números, `-` y `.`; empieza por letra o número.
fn nombre_valido(nombre: &str) -> bool {
    nombre
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && nombre
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '.'))
}

/// `propietario/nombre` de Hugging Face (las mismas reglas que `locall-core`).
fn repo_valido(repo: &str) -> bool {
    let Some((dueno, nombre)) = repo.split_once('/') else {
        return false;
    };
    [dueno, nombre].iter().all(|parte| {
        !parte.is_empty()
            && parte.len() <= 96
            && !parte.starts_with('.')
            && parte
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    })
}

/// Una ruta relativa a un `.gguf`, sin `..`, sin barras de Windows y sin empezar por `/`.
fn ruta_valida(ruta: &str) -> bool {
    ruta.ends_with(".gguf")
        && !ruta.starts_with('/')
        && !ruta.contains('\\')
        && ruta
            .split('/')
            .all(|parte| !parte.is_empty() && parte != "." && parte != "..")
        && !ruta.chars().any(char::is_control)
}

fn hexadecimal(texto: &str, largo: usize) -> bool {
    texto.len() == largo
        && texto
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

// ─── Lectura de JSON ──────────────────────────────────────────────────────────

fn invalido(donde: &str, que: &str) -> ErrorCatalogo {
    ErrorCatalogo::NoValido(format!("{donde}: {que}"))
}

fn objeto<'a>(valor: &'a Value, donde: &str) -> Result<&'a Map<String, Value>, ErrorCatalogo> {
    valor
        .as_object()
        .ok_or_else(|| invalido(donde, "debe ser un objeto"))
}

fn texto(
    o: &Map<String, Value>,
    campo: &str,
    donde: &str,
    maximo: usize,
) -> Result<String, ErrorCatalogo> {
    let t = o
        .get(campo)
        .and_then(Value::as_str)
        .ok_or_else(|| invalido(donde, &format!("falta «{campo}» (texto)")))?;
    if t.trim().is_empty() {
        return Err(invalido(donde, &format!("«{campo}» está vacío")));
    }
    if t.chars().count() > maximo {
        return Err(invalido(donde, &format!("«{campo}» es demasiado largo")));
    }
    if t.chars().any(|c| c.is_control() && c != '\n') {
        return Err(invalido(
            donde,
            &format!("«{campo}» tiene caracteres de control"),
        ));
    }
    Ok(t.to_owned())
}

fn numero(o: &Map<String, Value>, campo: &str, donde: &str) -> Result<u64, ErrorCatalogo> {
    o.get(campo)
        .and_then(Value::as_u64)
        .ok_or_else(|| invalido(donde, &format!("falta «{campo}» (número entero positivo)")))
}

fn lista<'a>(
    o: &'a Map<String, Value>,
    campo: &str,
    donde: &str,
    maximo: usize,
) -> Result<&'a Vec<Value>, ErrorCatalogo> {
    let l = o
        .get(campo)
        .and_then(Value::as_array)
        .ok_or_else(|| invalido(donde, &format!("falta «{campo}» (lista)")))?;
    if l.len() > maximo {
        return Err(invalido(
            donde,
            &format!("«{campo}» tiene demasiados elementos (máximo {maximo})"),
        ));
    }
    Ok(l)
}

// ─── Errores ──────────────────────────────────────────────────────────────────

/// Lo que puede salir mal al leer un catálogo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorCatalogo {
    /// El archivo pasa de [`TAMANO_MAX`].
    DemasiadoGrande(usize),
    /// La clave pública no es una clave de minisign válida.
    ClaveNoValida,
    /// El archivo de firma no tiene el formato de minisign.
    FirmaIlegible,
    /// La firma no corresponde a este catálogo con esta clave (modificado, firmado por otro o formato antiguo).
    FirmaIncorrecta,
    /// El JSON está mal escrito.
    Json(String),
    /// Un formato más nuevo que el que entiende este Locall.
    FormatoNuevo(u64),
    /// Un campo no cumple las reglas.
    NoValido(String),
}

impl fmt::Display for ErrorCatalogo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorCatalogo::DemasiadoGrande(n) => write!(
                f,
                "el catálogo ocupa {n} bytes, más de lo permitido ({TAMANO_MAX}); no se lee"
            ),
            ErrorCatalogo::ClaveNoValida => write!(f, "la clave pública del catálogo no es válida"),
            ErrorCatalogo::FirmaIlegible => write!(f, "la firma del catálogo no se entiende"),
            ErrorCatalogo::FirmaIncorrecta => write!(
                f,
                "la firma del catálogo no es válida: puede estar modificado o no venir de Locall; no se usa"
            ),
            ErrorCatalogo::Json(e) => write!(f, "el catálogo no es JSON válido: {e}"),
            ErrorCatalogo::FormatoNuevo(n) => write!(
                f,
                "el catálogo usa el formato {n} y este Locall entiende hasta el {FORMATO}: actualiza Locall"
            ),
            ErrorCatalogo::NoValido(m) => write!(f, "catálogo no válido: {m}"),
        }
    }
}

impl std::error::Error for ErrorCatalogo {}

#[cfg(test)]
mod pruebas;
