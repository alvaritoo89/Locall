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
    /// `locall pull <repo> [archivo] [--revision R] [--id ID]`. Sin archivo, lista los GGUF del repositorio.
    Instalar {
        repo: String,
        archivo: Option<String>,
        revision: Option<String>,
        id: Option<String>,
    },
    /// `locall import <ruta> [--id ID]`
    Importar {
        ruta: String,
        id: Option<String>,
    },
    /// `locall rm <id> [--yes]`
    Eliminar {
        id: String,
        sin_preguntar: bool,
    },
    /// `locall run <id> [pregunta…]`
    Conversar {
        id: String,
        pregunta: Option<String>,
    },
}

pub const AYUDA: &str = "\
Locall: modelos de IA en tu propio ordenador.

Uso: locall <orden> [argumentos]

Modelos:
  pull <repositorio> [archivo]   Instala un modelo de Hugging Face (sin archivo: lista los que tiene)
       --revision <r>            Rama, etiqueta o commit (por defecto, main)
       --id <nombre>             Nombre para el modelo en Locall
  import <ruta.gguf> [--id <n>]  Incorpora un modelo que ya tienes en el disco
  run <modelo> [pregunta]        Conversa con un modelo (sin pregunta: chat; Ctrl+C corta la respuesta)
  list                           Modelos instalados
  show <modelo>                  Detalles de un modelo
  rm <modelo> [--yes]            Elimina un modelo (--yes: sin preguntar)

Locall:
  status                         Si Locall está abierto y qué modelo tiene cargado
  serve                          Abre Locall y su API local en esta ventana (Ctrl+C para parar)
  stop                           Cierra Locall
  version                        Versión
  help                           Esta ayuda

Locall se abre solo, en segundo plano, cuando una orden lo necesita, y se cierra solo
cuando lleva un rato sin usarse.

Ejemplos:
  locall pull Qwen/Qwen3-4B-GGUF Qwen3-4B-Q4_K_M.gguf
  locall run qwen3-4b-q4_k_m
  locall run qwen3-4b-q4_k_m \"¿Qué es Rust?\"

Variables de entorno:
  LOCALL_DATOS          Carpeta de datos (por defecto %LOCALAPPDATA%\\Locall)
  LOCALL_MOTOR          Ruta de llama-server.exe (por defecto, motor\\llama-server.exe junto a locall.exe)
  LOCALL_HUGGING_FACE   Dirección de Hugging Face (por defecto https://huggingface.co; para espejos)
";

/// Interpreta los argumentos (sin el nombre del programa).
pub fn interpretar(argumentos: impl IntoIterator<Item = OsString>) -> Result<Orden, ErrorCli> {
    use lexopt::prelude::*;

    let mut lector = lexopt::Parser::from_args(argumentos);
    let mut orden: Option<String> = None;
    let mut resto: Vec<String> = Vec::new();
    let mut revision: Option<String> = None;
    let mut id: Option<String> = None;
    let mut sin_preguntar = false;
    let texto = |valor: OsString| {
        valor
            .into_string()
            .map_err(|_| ErrorCli::Uso("hay un argumento con caracteres no válidos".into()))
    };
    while let Some(argumento) = lector.next().map_err(traducir)? {
        match argumento {
            Short('h') | Long("help") => return Ok(Orden::Ayuda),
            Short('V') | Long("version") => return Ok(Orden::Version),
            Long("revision") => revision = Some(texto(lector.value().map_err(traducir)?)?),
            Long("id") => id = Some(texto(lector.value().map_err(traducir)?)?),
            Short('y') | Long("yes") => sin_preguntar = true,
            Value(valor) => {
                let valor = texto(valor)?;
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
    // Cada opción solo vale para algunas órdenes.
    let permitidas: &[&str] = match orden.as_str() {
        "pull" => &["--revision", "--id"],
        "import" => &["--id"],
        "rm" => &["--yes"],
        _ => &[],
    };
    for (puesta, nombre) in [
        (revision.is_some(), "--revision"),
        (id.is_some(), "--id"),
        (sin_preguntar, "--yes"),
    ] {
        if puesta && !permitidas.contains(&nombre) {
            return Err(ErrorCli::Uso(format!("«{orden}» no admite «{nombre}»")));
        }
    }

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
    // Exactamente un argumento (el modelo, la ruta…).
    let uno = |falta: &str| match resto.as_slice() {
        [x] => Ok(x.clone()),
        [] => Err(ErrorCli::Uso(format!("falta {falta}"))),
        [_, sobra, ..] => Err(ErrorCli::Uso(format!("sobra «{sobra}»"))),
    };
    match orden.as_str() {
        "help" | "ayuda" => sin_argumentos(Orden::Ayuda),
        "version" => sin_argumentos(Orden::Version),
        "serve" => sin_argumentos(Orden::Servir),
        "status" => sin_argumentos(Orden::Estado),
        "list" | "ls" => sin_argumentos(Orden::Lista),
        "stop" => sin_argumentos(Orden::Parar),
        "show" => Ok(Orden::Mostrar(uno("el modelo: locall show <modelo>")?)),
        "rm" => Ok(Orden::Eliminar {
            id: uno("el modelo: locall rm <modelo>")?,
            sin_preguntar,
        }),
        "run" => match resto.as_slice() {
            [] => Err(ErrorCli::Uso(
                "falta el modelo: locall run <modelo> [pregunta]".into(),
            )),
            [id] => Ok(Orden::Conversar {
                id: id.clone(),
                pregunta: None,
            }),
            // La pregunta puede ir entre comillas o no: las palabras se juntan.
            [id, palabras @ ..] => Ok(Orden::Conversar {
                id: id.clone(),
                pregunta: Some(palabras.join(" ")),
            }),
        },
        "import" => Ok(Orden::Importar {
            ruta: uno("la ruta del archivo: locall import <ruta.gguf>")?,
            id,
        }),
        "pull" => match resto.as_slice() {
            [repo] => Ok(Orden::Instalar {
                repo: repo.clone(),
                archivo: None,
                revision,
                id,
            }),
            [repo, archivo] => Ok(Orden::Instalar {
                repo: repo.clone(),
                archivo: Some(archivo.clone()),
                revision,
                id,
            }),
            [] => Err(ErrorCli::Uso(
                "falta el repositorio: locall pull <repositorio> [archivo]".into(),
            )),
            [_, _, sobra, ..] => Err(ErrorCli::Uso(format!("sobra «{sobra}»"))),
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
    fn ordenes_de_modelos() {
        let instalar =
            |archivo: Option<&str>, revision: Option<&str>, id: Option<&str>| Orden::Instalar {
                repo: "dueno/repo".into(),
                archivo: archivo.map(Into::into),
                revision: revision.map(Into::into),
                id: id.map(Into::into),
            };
        assert_eq!(de("pull dueno/repo").unwrap(), instalar(None, None, None));
        assert_eq!(
            de("pull dueno/repo m.gguf --revision v2 --id mio").unwrap(),
            instalar(Some("m.gguf"), Some("v2"), Some("mio"))
        );
        assert_eq!(
            de("pull dueno/repo m.gguf --revision=v2").unwrap(),
            instalar(Some("m.gguf"), Some("v2"), None)
        );
        assert_eq!(
            de(r"import D:\m.gguf --id mio").unwrap(),
            Orden::Importar {
                ruta: r"D:\m.gguf".into(),
                id: Some("mio".into())
            }
        );
        assert_eq!(
            de("rm qwen -y").unwrap(),
            Orden::Eliminar {
                id: "qwen".into(),
                sin_preguntar: true
            }
        );
        assert_eq!(
            de("rm qwen").unwrap(),
            Orden::Eliminar {
                id: "qwen".into(),
                sin_preguntar: false
            }
        );
    }

    #[test]
    fn conversar() {
        assert_eq!(
            de("run qwen").unwrap(),
            Orden::Conversar {
                id: "qwen".into(),
                pregunta: None
            }
        );
        assert_eq!(
            de("run qwen qué es Rust").unwrap(),
            Orden::Conversar {
                id: "qwen".into(),
                pregunta: Some("qué es Rust".into())
            }
        );
        assert!(
            de("run")
                .unwrap_err()
                .to_string()
                .contains("falta el modelo")
        );
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
            ("pull", "falta el repositorio"),
            ("pull a/b c d", "sobra «d»"),
            ("pull a/b --revision", "le falta su valor"),
            ("list --id x", "«list» no admite «--id»"),
            ("import x.gguf --yes", "«import» no admite «--yes»"),
            ("rm", "falta el modelo"),
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
