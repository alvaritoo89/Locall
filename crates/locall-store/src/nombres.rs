//! Safe names for the store's files and folders.
//!
//! Like [`crate::Huella`], they are *newtypes*: they only exist if they are valid. This way no name coming from
//! outside (from the catalog, from Hugging Face, from the user…) can escape the store folder or clash with Windows'
//! rules.

use std::fmt;

use crate::ErrorAlmacen;

/// Names Windows reserves for devices, which cannot be used as file names (not even with an extension).
const RESERVADOS_WINDOWS: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Maximum length of a name (Windows allows 255 characters per path component).
const MAX_LONGITUD: usize = 200;

fn invalido(nombre: &str, motivo: &'static str) -> ErrorAlmacen {
    ErrorAlmacen::NombreInvalido {
        nombre: nombre.to_string(),
        motivo,
    }
}

/// Checks common to any file or folder name on Windows.
fn validar_componente(nombre: &str) -> Result<(), ErrorAlmacen> {
    if nombre.is_empty() {
        return Err(invalido(nombre, "está vacío"));
    }
    if nombre.chars().count() > MAX_LONGITUD {
        return Err(invalido(nombre, "es demasiado largo"));
    }
    if nombre == "." || nombre == ".." {
        return Err(invalido(nombre, "no puede ser «.» ni «..»"));
    }
    if nombre.chars().any(|c| {
        matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control()
    }) {
        return Err(invalido(
            nombre,
            "contiene caracteres no permitidos (/ \\ : * ? \" < > | o de control)",
        ));
    }
    if nombre.ends_with('.') || nombre.ends_with(' ') {
        return Err(invalido(
            nombre,
            "no puede terminar en punto ni en espacio (Windows los elimina)",
        ));
    }
    let base = nombre.split('.').next().unwrap_or("").to_ascii_uppercase();
    if RESERVADOS_WINDOWS.contains(&base.as_str()) {
        return Err(invalido(nombre, "es un nombre reservado de Windows"));
    }
    Ok(())
}

// ─── NombreArchivo ────────────────────────────────────────────────────────────

/// The original name of a model file, for example `Qwen3-4B-Q4_K_M.gguf`.
/// It must be a single component (no folders) and end in `.gguf` (ADR 0004).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NombreArchivo(String);

impl NombreArchivo {
    pub fn nuevo(nombre: &str) -> Result<NombreArchivo, ErrorAlmacen> {
        validar_componente(nombre)?;
        if !nombre.to_ascii_lowercase().ends_with(".gguf") || nombre.len() <= ".gguf".len() {
            return Err(invalido(nombre, "debe ser un archivo .gguf"));
        }
        Ok(NombreArchivo(nombre.to_string()))
    }

    pub fn como_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NombreArchivo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// ─── Segmento ─────────────────────────────────────────────────────────────────

/// A store folder name: the model identifier (`qwen3-4b`) or the revision (`bc640142`).
///
/// Stricter than [`NombreArchivo`]: only lowercase letters, digits, `-`, `_` and `.` (not starting with a dot), so it
/// is equally valid on Windows, Linux and macOS.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Segmento(String);

impl Segmento {
    pub fn nuevo(texto: &str) -> Result<Segmento, ErrorAlmacen> {
        validar_componente(texto)?;
        if texto.starts_with('.') {
            return Err(invalido(texto, "no puede empezar por punto"));
        }
        if !texto.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'.')
        }) {
            return Err(invalido(
                texto,
                "solo admite minúsculas, números, «-», «_» y «.»",
            ));
        }
        Ok(Segmento(texto.to_string()))
    }

    pub fn como_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Segmento {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn acepta_nombres_de_modelo_normales() {
        for bueno in [
            "Qwen3-4B-Q4_K_M.gguf",
            "qwen2.5-7b-instruct-q4_k_m-00001-of-00002.gguf",
            "mmproj-F16.GGUF",
        ] {
            assert!(
                NombreArchivo::nuevo(bueno).is_ok(),
                "debería aceptar «{bueno}»"
            );
        }
    }

    #[test]
    fn rechaza_nombres_peligrosos_o_invalidos() {
        for malo in [
            "",
            ".gguf",
            "..",
            "modelo.txt",
            "..\\..\\Windows\\modelo.gguf",
            "carpeta/modelo.gguf",
            "C:modelo.gguf",
            "CON.gguf",
            "nul.gguf",
            "modelo.gguf ",
            "modelo.gguf.",
            "mo\u{0}delo.gguf",
            "modelo?.gguf",
        ] {
            assert!(
                NombreArchivo::nuevo(malo).is_err(),
                "debería rechazar «{malo:?}»"
            );
        }
    }

    #[test]
    fn valida_los_segmentos() {
        for bueno in ["qwen3-4b", "bc640142", "qwen2.5-7b-instruct"] {
            assert!(Segmento::nuevo(bueno).is_ok(), "debería aceptar «{bueno}»");
        }
        for malo in ["", "Qwen3", "..", ".oculto", "a b", "a/b", "aux", "ñandú"] {
            assert!(Segmento::nuevo(malo).is_err(), "debería rechazar «{malo}»");
        }
    }
}
