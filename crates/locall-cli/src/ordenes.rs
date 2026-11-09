//! Interpretar la línea de comandos con `lexopt`. La ayuda y todos los errores son nuestros, en español.

use std::ffi::OsString;

use crate::cliente::ErrorCli;

/// Lo que ha pedido el usuario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Orden {
    Ayuda,
    Version,
    /// `locall serve` (lo atiende el programa, que tiene el servidor).
    Servir,
    /// `locall status`
    Estado,
    /// `locall list`
    Lista,
    /// `locall show <id>`
    Mostrar(String),
    /// `locall stop`
    Parar,
}

pub const AYUDA: &str = "\
Locall: modelos de IA en tu propio ordenador.

Uso: locall <orden> [argumentos]

Órdenes:
  list              Modelos instalados
  show <modelo>     Detalles de un modelo
  status            Si Locall está abierto y qué modelo tiene cargado
  serve             Abre Locall y su API local en esta ventana (Ctrl+C para parar)
  stop              Cierra Locall
  version           Versión
  help              Esta ayuda

Locall se abre solo, en segundo plano, cuando una orden lo necesita, y se cierra solo
cuando lleva un rato sin usarse.

Variables de entorno:
  LOCALL_DATOS      Carpeta de datos (por defecto %LOCALAPPDATA%\\Locall)
  LOCALL_MOTOR      Ruta de llama-server.exe (por defecto, motor\\llama-server.exe junto a locall.exe)
";

/// Interpreta los argumentos (sin el nombre del programa).
pub fn interpretar(argumentos: impl IntoIterator<Item = OsString>) -> Result<Orden, ErrorCli> {
    use lexopt::prelude::*;

    let mut lector = lexopt::Parser::from_args(argumentos);
    let mut orden: Option<String> = None;
    let mut resto: Vec<String> = Vec::new();
    while let Some(argumento) = lector.next().map_err(traducir)? {
        match argumento {
            Short('h') | Long("help") => return Ok(Orden::Ayuda),
            Short('V') | Long("version") => return Ok(Orden::Version),
            Value(valor) => {
                let valor = valor.into_string().map_err(|_| {
                    ErrorCli::Uso("hay un argumento con caracteres no válidos".into())
                })?;
                if orden.is_none() {
                    orden = Some(valor);
                } else {
                    resto.push(valor);
                }
            }
            Short(c) => return Err(ErrorCli::Uso(format!("opción desconocida «-{c}»"))),
            Long(nombre) => return Err(ErrorCli::Uso(format!("opción desconocida «--{nombre}»"))),
        }
    }

    let Some(orden) = orden else {
        return Ok(Orden::Ayuda);
    };
    let sin_argumentos = |o: Orden| {
        if resto.is_empty() {
            Ok(o)
        } else {
            Err(ErrorCli::Uso(format!(
                "«{orden}» no lleva argumentos (sobra «{}»)",
                resto[0]
            )))
        }
    };
    match orden.as_str() {
        "help" | "ayuda" => sin_argumentos(Orden::Ayuda),
        "version" => sin_argumentos(Orden::Version),
        "serve" => sin_argumentos(Orden::Servir),
        "status" => sin_argumentos(Orden::Estado),
        "list" | "ls" => sin_argumentos(Orden::Lista),
        "stop" => sin_argumentos(Orden::Parar),
        "show" => match resto.as_slice() {
            [id] => Ok(Orden::Mostrar(id.clone())),
            [] => Err(ErrorCli::Uso(
                "falta el modelo: locall show <modelo>".into(),
            )),
            [_, sobra, ..] => Err(ErrorCli::Uso(format!(
                "«show» lleva un solo modelo (sobra «{sobra}»)"
            ))),
        },
        otra => Err(ErrorCli::Uso(format!("orden desconocida «{otra}»"))),
    }
}

/// Los errores de `lexopt`, en español.
fn traducir(error: lexopt::Error) -> ErrorCli {
    ErrorCli::Uso(match error {
        lexopt::Error::MissingValue { option: Some(o) } => format!("a «{o}» le falta su valor"),
        lexopt::Error::UnexpectedValue { option, .. } => format!("«{option}» no lleva valor"),
        lexopt::Error::NonUnicodeValue(_) => "hay un argumento con caracteres no válidos".into(),
        otro => format!("argumentos no válidos ({otro})"),
    })
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn de(texto: &str) -> Result<Orden, ErrorCli> {
        interpretar(texto.split_whitespace().map(OsString::from))
    }

    #[test]
    fn reconoce_las_ordenes() {
        assert_eq!(de("").unwrap(), Orden::Ayuda);
        assert_eq!(de("help").unwrap(), Orden::Ayuda);
        assert_eq!(de("--help").unwrap(), Orden::Ayuda);
        assert_eq!(de("list -h").unwrap(), Orden::Ayuda);
        assert_eq!(de("-V").unwrap(), Orden::Version);
        assert_eq!(de("version").unwrap(), Orden::Version);
        assert_eq!(de("serve").unwrap(), Orden::Servir);
        assert_eq!(de("status").unwrap(), Orden::Estado);
        assert_eq!(de("list").unwrap(), Orden::Lista);
        assert_eq!(de("ls").unwrap(), Orden::Lista);
        assert_eq!(de("stop").unwrap(), Orden::Parar);
        assert_eq!(de("show qwen").unwrap(), Orden::Mostrar("qwen".into()));
    }

    #[test]
    fn explica_los_errores_en_espanol() {
        let casos = [
            ("lanzar", "orden desconocida «lanzar»"),
            ("list extra", "«list» no lleva argumentos (sobra «extra»)"),
            ("show", "falta el modelo"),
            ("show a b", "sobra «b»"),
            ("--rapido", "opción desconocida «--rapido»"),
            ("list -x", "opción desconocida «-x»"),
        ];
        for (texto, esperado) in casos {
            let error = de(texto).unwrap_err().to_string();
            assert!(error.contains(esperado), "{texto}: {error}");
            assert!(
                error.contains("locall help"),
                "siempre indica dónde mirar: {error}"
            );
        }
    }
}
