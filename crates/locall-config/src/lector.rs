//! Reader for a small subset of TOML: `key = value` lines and `#` comments.
//!
//! What it understands (and it is valid TOML, so one day it can be switched to the `toml` crate without touching the
//! files):
//!
//! ```toml
//! # comment
//! contexto = 4096          # integers (4_096 is accepted)
//! hilos = "auto"           # texts in double quotes (with \\ \" \n \t)
//! carpeta = 'C:\Modelos'   # texts in single quotes: taken as is, ideal for Windows paths
//! ```
//!
//! What it does NOT understand (and it says so with the line number): `[...]` sections, arrays, tables, dates,
//! decimals, multi-line texts. Locall does not need them yet.

use std::fmt;

/// A value that was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Valor {
    Texto(String),
    Entero(i64),
}

impl fmt::Display for Valor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Valor::Texto(t) => write!(f, "\"{t}\""),
            Valor::Entero(n) => write!(f, "{n}"),
        }
    }
}

/// A `key = value` line, with its line number (starting at 1) for the messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entrada {
    pub linea: usize,
    pub clave: String,
    pub valor: Valor,
}

/// Syntax error, with the line number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorSintaxis {
    pub linea: usize,
    pub mensaje: String,
}

/// Reads all the entries of the text, in order. A repeated key is an error (it is in TOML too).
pub fn leer(texto: &str) -> Result<Vec<Entrada>, ErrorSintaxis> {
    let mut entradas: Vec<Entrada> = Vec::new();
    // `lines()` accepts Windows (\r\n) and Linux (\n) line endings.
    for (i, linea) in texto.lines().enumerate() {
        let numero = i + 1;
        let error = |mensaje: String| ErrorSintaxis {
            linea: numero,
            mensaje,
        };
        // Windows Notepad sometimes puts a "byte order mark" (BOM) at the start.
        let linea = if numero == 1 {
            linea.trim_start_matches('\u{feff}')
        } else {
            linea
        };
        let linea = linea.trim();
        if linea.is_empty() || linea.starts_with('#') {
            continue;
        }
        if linea.starts_with('[') {
            return Err(error("las secciones [...] no se admiten todavía".into()));
        }
        let Some((clave, resto)) = linea.split_once('=') else {
            return Err(error("se esperaba «clave = valor»".into()));
        };
        let clave = clave.trim();
        if clave.is_empty()
            || !clave
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(error(format!("nombre de ajuste no válido: «{clave}»")));
        }
        let valor = leer_valor(resto.trim()).map_err(error)?;
        if entradas.iter().any(|e| e.clave == clave) {
            return Err(error(format!("«{clave}» aparece dos veces")));
        }
        entradas.push(Entrada {
            linea: numero,
            clave: clave.to_owned(),
            valor,
        });
    }
    Ok(entradas)
}

/// Reads the value (what comes after the `=`), with an optional comment at the end.
fn leer_valor(texto: &str) -> Result<Valor, String> {
    let (valor, resto) = match texto.chars().next() {
        None => return Err("falta el valor tras el «=»".into()),
        Some('\'') => texto_literal(texto)?,
        Some('"') => texto_basico(texto)?,
        Some(_) => {
            // No quotes: a number. It ends at a space or a comment.
            let fin = texto.find(['#', ' ', '\t']).unwrap_or(texto.len());
            (entero(&texto[..fin])?, &texto[fin..])
        }
    };
    let resto = resto.trim_start();
    if !resto.is_empty() && !resto.starts_with('#') {
        return Err(format!("sobra texto tras el valor: «{resto}»"));
    }
    Ok(valor)
}

/// `'...'`: the content as is, without escape sequences.
fn texto_literal(texto: &str) -> Result<(Valor, &str), String> {
    let cuerpo = &texto[1..];
    let fin = cuerpo
        .find('\'')
        .ok_or_else(|| "falta la comilla simple de cierre".to_string())?;
    Ok((Valor::Texto(cuerpo[..fin].to_owned()), &cuerpo[fin + 1..]))
}

/// `"..."`: accepts `\\`, `\"`, `\n` and `\t`.
fn texto_basico(texto: &str) -> Result<(Valor, &str), String> {
    let mut resultado = String::new();
    let mut caracteres = texto[1..].char_indices();
    while let Some((i, c)) = caracteres.next() {
        match c {
            '"' => return Ok((Valor::Texto(resultado), &texto[1 + i + 1..])),
            '\\' => {
                match caracteres.next().map(|(_, c)| c) {
                    Some('\\') => resultado.push('\\'),
                    Some('"') => resultado.push('"'),
                    Some('n') => resultado.push('\n'),
                    Some('t') => resultado.push('\t'),
                    _ => {
                        return Err("secuencia «\\» no válida; para rutas de Windows usa comillas simples: \
                         'C:\\Modelos'"
                        .into());
                    }
                }
            }
            c => resultado.push(c),
        }
    }
    Err("falta la comilla doble de cierre".into())
}

/// An integer, with an optional sign and `_` between digits (`4_096`).
fn entero(texto: &str) -> Result<Valor, String> {
    let sin_guiones: String = texto.chars().filter(|&c| c != '_').collect();
    let valido = !texto.starts_with('_') && !texto.ends_with('_') && !texto.contains("__");
    match sin_guiones.parse::<i64>() {
        Ok(n) if valido => Ok(Valor::Entero(n)),
        _ => Err(format!(
            "«{texto}» no es un número; si es un texto, ponlo entre comillas"
        )),
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn una(texto: &str) -> Entrada {
        let mut entradas = leer(texto).unwrap();
        assert_eq!(entradas.len(), 1);
        entradas.remove(0)
    }

    #[test]
    fn lee_numeros_y_textos() {
        assert_eq!(una("contexto = 4096").valor, Valor::Entero(4096));
        assert_eq!(una("contexto=4_096").valor, Valor::Entero(4096));
        assert_eq!(una("a = -3").valor, Valor::Entero(-3));
        assert_eq!(una(r#"hilos = "auto""#).valor, Valor::Texto("auto".into()));
        assert_eq!(
            una(r"carpeta = 'C:\Modelos\Locall'").valor,
            Valor::Texto(r"C:\Modelos\Locall".into())
        );
        assert_eq!(
            una(r#"carpeta = "C:\\Modelos""#).valor,
            Valor::Texto(r"C:\Modelos".into())
        );
    }

    #[test]
    fn ignora_comentarios_y_lineas_vacias() {
        let texto = "# título\n\n  contexto = 2048   # comentario\r\nhilos = 'auto' # otro\n";
        let entradas = leer(texto).unwrap();
        assert_eq!(entradas.len(), 2);
        assert_eq!(entradas[0].linea, 3);
        assert_eq!(entradas[1].linea, 4);
        // A # inside a text is not a comment.
        assert_eq!(una("nombre = 'a#b'").valor, Valor::Texto("a#b".into()));
    }

    #[test]
    fn acepta_la_marca_del_bloc_de_notas() {
        assert_eq!(una("\u{feff}contexto = 1").clave, "contexto");
    }

    #[test]
    fn explica_los_errores_con_su_linea() {
        let casos = [
            ("\n\nsolo texto", 3, "clave = valor"),
            ("contexto = ", 1, "falta el valor"),
            ("contexto = cuatro", 1, "no es un número"),
            ("hilos = \"auto", 1, "comilla doble"),
            ("carpeta = 'C:\\x", 1, "comilla simple"),
            (r#"carpeta = "C:\Modelos""#, 1, "comillas simples"),
            ("a = 1\na = 2", 2, "dos veces"),
            ("[motor]", 1, "secciones"),
            ("contexto = 4096 basura", 1, "sobra texto"),
            ("mi clave = 1", 1, "no válido"),
            ("a = 1__0", 1, "no es un número"),
        ];
        for (texto, linea, fragmento) in casos {
            let error = leer(texto).unwrap_err();
            assert_eq!(error.linea, linea, "{texto:?}");
            assert!(
                error.mensaje.contains(fragmento),
                "{texto:?}: {}",
                error.mensaje
            );
        }
    }
}
