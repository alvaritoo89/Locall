//! El token de la API: 32 bytes aleatorios en hexadecimal, guardados en `%LOCALAPPDATA%\Locall\token`.
//!
//! Esa carpeta solo la puede leer el usuario de Windows (y los administradores). La CLI y la app leen el token de
//! ahí; para otras herramientas, el usuario lo copia como "API key".

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

/// Lee el token; si no existe (o no es válido), crea uno nuevo.
pub fn leer_o_crear(ruta: &Path) -> io::Result<String> {
    if let Some(token) = leer(ruta)? {
        return Ok(token);
    }
    if let Some(carpeta) = ruta.parent() {
        fs::create_dir_all(carpeta)?;
    }
    let token = nuevo()?;
    // Si el archivo existe pero no es válido, se reemplaza.
    let _ = fs::remove_file(ruta);
    match OpenOptions::new().write(true).create_new(true).open(ruta) {
        Ok(mut archivo) => {
            archivo.write_all(token.as_bytes())?;
            Ok(token)
        }
        // Otro proceso lo acaba de crear a la vez: usar el suyo.
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => leer(ruta)?.ok_or(e),
        Err(e) => Err(e),
    }
}

/// El token guardado, si es válido.
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

/// Compara sin revelar por el tiempo cuántos caracteres coinciden (las comparaciones normales paran en el primer
/// carácter distinto, y un atacante podría medirlo para adivinar el token poco a poco).
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

        // Uno estropeado se reemplaza.
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
