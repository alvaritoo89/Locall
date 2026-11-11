//! The API token: 32 random bytes in hexadecimal, stored in `%LOCALAPPDATA%\Locall\token`.
//!
//! Only the Windows user (and administrators) can read that folder. The CLI and the app read the token from there;
//! for other tools, the user copies it as the "API key".

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

/// Reads the token; if it does not exist (or is not valid), creates a new one.
pub fn leer_o_crear(ruta: &Path) -> io::Result<String> {
    if let Some(token) = leer(ruta)? {
        return Ok(token);
    }
    if let Some(carpeta) = ruta.parent() {
        fs::create_dir_all(carpeta)?;
    }
    let token = nuevo()?;
    // If the file exists but is not valid, it is replaced.
    let _ = fs::remove_file(ruta);
    match OpenOptions::new().write(true).create_new(true).open(ruta) {
        Ok(mut archivo) => {
            archivo.write_all(token.as_bytes())?;
            Ok(token)
        }
        // Another process just created it at the same time: use theirs.
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => leer(ruta)?.ok_or(e),
        Err(e) => Err(e),
    }
}

/// The saved token, if it is valid.
pub fn leer(ruta: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(ruta) {
        Ok(texto) => {
            let token = texto.trim();
            Ok(es_valido(token).then(|| token.to_owned()))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn es_valido(token: &str) -> bool {
    token.len() == 64 && token.chars().all(|c| c.is_ascii_hexdigit())
}

fn nuevo() -> io::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Compares without revealing through timing how many characters match (normal comparisons stop at the first
/// differing character, and an attacker could measure that to guess the token bit by bit).
pub(crate) fn iguales(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn crea_uno_y_despues_lo_reutiliza() {
        let carpeta = std::env::temp_dir().join(format!("locall-token-{}", std::process::id()));
        let _ = fs::remove_dir_all(&carpeta);
        let ruta = carpeta.join("token");

        let primero = leer_o_crear(&ruta).unwrap();
        assert!(es_valido(&primero));
        assert_eq!(leer_o_crear(&ruta).unwrap(), primero, "se reutiliza");

        // A broken one is replaced.
        fs::write(&ruta, "esto no es un token").unwrap();
        let nuevo = leer_o_crear(&ruta).unwrap();
        assert!(es_valido(&nuevo));
        assert_ne!(nuevo, primero);
        let _ = fs::remove_dir_all(&carpeta);
    }

    #[test]
    fn compara_tokens() {
        assert!(iguales("abc", "abc"));
        assert!(!iguales("abc", "abd"));
        assert!(!iguales("abc", "abcd"));
        assert!(!iguales("", "a"));
    }
}
