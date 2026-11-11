//! # locall-catalogo
//!
//! Locall's **catalog of compatible models** (ADR 0022): which models Locall recommends, exactly where they are
//! downloaded from (repository, pinned commit, files with their sha256) and what they are for.
//!
//! - The catalog is a JSON file (`catalogo/catalogo.json` in the repository) **signed with minisign** by whoever
//!   publishes Locall (`catalogo.json.minisig`). The app and the CLI ship the public key ([`CLAVE_PUBLICA`]).
//! - [`verificar`] checks **the signature first** and only then reads the content: an unsigned file is never
//!   interpreted. Then it validates field by field, with size limits.
//! - Being in the catalog means **compatible**: the header of the exact file (that commit) was read and the engine
//!   named in `checked_engine` recognizes its architecture and data types. It does **not** mean chatting with it was
//!   tested: the app and the website say so.
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
//! Fields this version does not know are ignored (so a newer catalog with extra fields still works), but a `format`
//! greater than [`FORMATO`] is an error: that catalog needs a newer Locall.

use std::collections::BTreeSet;
use std::fmt;

use minisign_verify::{PublicKey, Signature};
use serde_json::{Map, Value};

/// The format version this Locall understands.
pub const FORMATO: u64 = 1;

/// The public key the official catalog is signed with (minisign, id `5C09624CD1482FE0`).
pub const CLAVE_PUBLICA: &str = "RWTgL0jRTGIJXDFqcD3bREOcjTX2CAHHzp1u+orTwHN5XscqH2mHvk3S";

/// Maximum catalog size: if someone serves a huge file, it is rejected without reading it.
pub const TAMANO_MAX: usize = 4 * 1024 * 1024;

const MODELOS_MAX: usize = 500;
const VARIANTES_MAX: usize = 16;
const ARCHIVOS_MAX: usize = 64;
const ETIQUETAS_MAX: usize = 12;
const NOMBRE_MAX: usize = 48;
const TEXTO_CORTO_MAX: usize = 120;
const DESCRIPCION_MAX: usize = 2000;

/// The catalog, already verified and validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalogo {
    /// When it was published (seconds since 1970). Used to never accept one older than the last one accepted.
    pub publicado: u64,
    pub modelos: Vec<ModeloCatalogo>,
}

/// A catalog model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeloCatalogo {
    /// Short name, the one typed in `locall pull`: `qwen3-4b`.
    pub nombre: String,
    /// Display name: `Qwen3 4B`.
    pub titulo: String,
    /// Who publishes it: `Qwen`.
    pub autor: String,
    /// What it is for, in Spanish.
    pub descripcion: String,
    /// Tags for the filters (`conversar`, `programar`…).
    pub etiquetas: Vec<String>,
    /// License (`apache-2.0`, `mit`…).
    pub licencia: String,
    /// Maximum context the model supports, in tokens.
    pub contexto_max: u64,
    /// The variants (quantizations). Exactly one is the default.
    pub variantes: Vec<Variante>,
}

/// A variant of a model: one specific GGUF file (or several, if it is split).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variante {
    /// `q4_k_m`, `q8_0`…
    pub nombre: String,
    pub por_defecto: bool,
    /// Hugging Face repository: `Qwen/Qwen3-4B-GGUF`.
    pub repo: String,
    /// Exact commit (40 lowercase hexadecimal characters).
    pub commit: String,
    /// Engine version compatibility was checked with (`b12189`).
    pub motor_comprobado: String,
    pub archivos: Vec<ArchivoCatalogo>,
}

/// A file of a variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivoCatalogo {
    /// Path inside the repository (may contain folders).
    pub ruta: String,
    pub bytes: u64,
    /// sha256 in hexadecimal (64 lowercase characters).
    pub sha256: String,
}

impl Variante {
    /// Space it takes on disk.
    pub fn bytes(&self) -> u64 {
        self.archivos.iter().map(|a| a.bytes).sum()
    }
}

impl ModeloCatalogo {
    /// The default variant (there is always one: validation guarantees it).
    pub fn por_defecto(&self) -> &Variante {
        self.variantes
            .iter()
            .find(|v| v.por_defecto)
            .unwrap_or(&self.variantes[0])
    }
}

impl Catalogo {
    /// Looks up what the person types: `qwen3-4b` (the default variant) or `qwen3-4b:q8_0`. Case-insensitive.
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

// ─── The catalog shipped inside Locall ────────────────────────────────────────

/// The official catalog as it was when this Locall was built (`catalogo/catalogo.json`), with its signature. Works
/// without internet and as a starting point: the app replaces it with a newer one if it downloads one with a valid
/// signature.
pub const CATALOGO_INCLUIDO: &[u8] = include_bytes!("../../../catalogo/catalogo.json");
/// The signature of [`CATALOGO_INCLUIDO`].
pub const FIRMA_INCLUIDA: &str = include_str!("../../../catalogo/catalogo.json.minisig");

/// The catalog shipped inside Locall, verified exactly like a downloaded one (with [`CLAVE_PUBLICA`]).
pub fn incluido() -> Result<Catalogo, ErrorCatalogo> {
    verificar(CATALOGO_INCLUIDO, FIRMA_INCLUIDA, CLAVE_PUBLICA)
}

// ─── Verifying ────────────────────────────────────────────────────────────────

/// Checks the signature of `datos` with `clave_publica` (minisign base64) and, if valid, reads and validates the
/// catalog.
///
/// Only signatures in minisign's current format are accepted (the file is hashed with BLAKE2b, minisign 0.12's
/// default); legacy-format signatures are rejected.
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
    // From here on, the content comes from whoever holds the private key.
    leer(datos)
}

/// Reads and validates a catalog **without checking the signature**. Only for the publisher (preparing and reviewing
/// a catalog before signing it) and for tests; the app always uses [`verificar`].
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

// ─── Field rules ──────────────────────────────────────────────────────────────

/// Short name: lowercase letters, digits, `-` and `.`; starts with a letter or digit.
fn nombre_valido(nombre: &str) -> bool {
    nombre
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && nombre
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '.'))
}

/// Hugging Face `owner/name` (the same rules as `locall-core`).
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

/// A relative path to a `.gguf`, without `..`, without Windows backslashes and not starting with `/`.
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

// ─── Reading JSON ─────────────────────────────────────────────────────────────

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

// ─── Errors ───────────────────────────────────────────────────────────────────

/// What can go wrong when reading a catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorCatalogo {
    /// The file exceeds [`TAMANO_MAX`].
    DemasiadoGrande(usize),
    /// The public key is not a valid minisign key.
    ClaveNoValida,
    /// The signature file is not in minisign's format.
    FirmaIlegible,
    /// The signature does not match this catalog with this key (modified, signed by someone else, or legacy format).
    FirmaIncorrecta,
    /// The JSON is malformed.
    Json(String),
    /// A newer format than this Locall understands.
    FormatoNuevo(u64),
    /// A field does not follow the rules.
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
