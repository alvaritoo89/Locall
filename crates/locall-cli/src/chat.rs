//! `locall run <modelo> [pregunta]`: conversar en la terminal.
//!
//! - Con una pregunta (o con texto llegando por la entrada, `echo hola | locall run qwen`): una respuesta y fin.
//! - Sin pregunta, en una terminal: un **chat**. La conversación se recuerda mientras dura (se envía entera en cada
//!   pregunta: el motor no guarda nada entre peticiones).
//! - **Ctrl+C mientras responde** corta esa respuesta y vuelve a preguntar; **Ctrl+C esperando** sale.
//!
//! Cortar funciona cerrando la conexión: el servidor lo nota y el motor deja de generar (Fase 3). Se comprueba entre
//! trozo y trozo, así que tarda como mucho lo que tarde en llegar el siguiente (un token, o un lote del prompt).

use std::io::{BufRead, BufReader, IsTerminal, Read, Write};

use serde_json::{Value, json};

use crate::cliente::{Cliente, ErrorCli};
use crate::modelos::codificar;

/// Resultado de una respuesta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Respuesta {
    pub texto: String,
    /// Se cortó con Ctrl+C.
    pub cortada: bool,
}

const AYUDA_CHAT: &str = "\
  /nuevo    Empieza una conversación nueva (olvida lo anterior)
  /salir    Termina (también Ctrl+C o Ctrl+Z y Enter)
  /ayuda    Esta ayuda
";

/// `locall run`
pub fn ejecutar(cliente: &Cliente, id: &str, pregunta: Option<&str>) -> Result<String, ErrorCli> {
    // Comprobar que el modelo existe antes de nada (error claro y rápido).
    cliente.get(&format!("/api/models/{}", codificar(id)))?;
    let _ = locall_system::capturar_ctrl_c(); // si no se puede, Ctrl+C cerrará el programa (lo de siempre)
    let _ = locall_system::ctrl_c_pulsado(); // bajar un aviso anterior

    let pregunta = match pregunta {
        Some(p) => Some(p.to_owned()),
        // Sin terminal en la entrada: lo que llegue por ella es la pregunta.
        None if !std::io::stdin().is_terminal() => {
            let mut texto = String::new();
            std::io::stdin()
                .read_to_string(&mut texto)
                .map_err(|e| ErrorCli::Uso(format!("no se pudo leer la pregunta: {e}")))?;
            Some(texto)
        }
        None => None,
    };

    let mut salida = std::io::stdout();
    match pregunta {
        Some(p) if p.trim().is_empty() => Err(ErrorCli::Uso("la pregunta está vacía".into())),
        Some(p) => {
            avisar_si_carga(cliente, id);
            let mensajes = [json!({"role": "user", "content": p.trim()})];
            let r = responder(cliente, id, &mensajes, &mut salida)?;
            Ok(if r.cortada {
                String::new()
            } else {
                "\n".into()
            })
        }
        None => {
            println!(
                "Conversando con «{id}». Escribe /ayuda para ver las órdenes, /salir (o Ctrl+C) para terminar."
            );
            avisar_si_carga(cliente, id);
            let entrada = std::io::stdin().lock();
            conversar(
                entrada,
                &mut salida,
                |mensajes, salida| responder(cliente, id, mensajes, salida),
                locall_system::ctrl_c_pulsado,
            )?;
            Ok(String::new())
        }
    }
}

/// Si el modelo no está cargado, avisa de que la primera respuesta tardará un poco más.
fn avisar_si_carga(cliente: &Cliente, id: &str) {
    if let Ok(estado) = cliente.get("/api/status")
        && !(estado["state"] == "ready" && estado["model"] == id)
    {
        let _ = writeln!(
            std::io::stderr(),
            "(Cargando el modelo; la primera respuesta tardará unos segundos…)"
        );
    }
}

/// El bucle del chat, separado de la red y de la consola para poder probarlo.
///
/// `responder` envía la conversación y escribe la respuesta; `interrumpido` dice si se ha pulsado Ctrl+C.
pub fn conversar<E: BufRead, S: Write>(
    mut entrada: E,
    salida: &mut S,
    mut responder: impl FnMut(&[Value], &mut S) -> Result<Respuesta, ErrorCli>,
    interrumpido: impl Fn() -> bool,
) -> Result<(), ErrorCli> {
    let mut historial: Vec<Value> = Vec::new();
    loop {
        let _ = write!(salida, "\n>>> ");
        let _ = salida.flush();
        let mut linea = String::new();
        let leidos = entrada.read_line(&mut linea).unwrap_or(0);
        if leidos == 0 || interrumpido() {
            let _ = writeln!(salida);
            return Ok(()); // Ctrl+C, Ctrl+Z o fin de la entrada
        }
        let texto = linea.trim();
        match texto {
            "" => continue,
            "/salir" | "/exit" | "/bye" => return Ok(()),
            "/ayuda" | "/help" | "/?" => {
                let _ = write!(salida, "{AYUDA_CHAT}");
                continue;
            }
            "/nuevo" | "/clear" => {
                historial.clear();
                let _ = writeln!(salida, "Conversación nueva.");
                continue;
            }
            otro if otro.starts_with('/') => {
                let _ = writeln!(salida, "Orden desconocida «{otro}». Escribe /ayuda.");
                continue;
            }
            _ => {}
        }
        historial.push(json!({"role": "user", "content": texto}));
        match responder(&historial, salida) {
            Ok(r) => {
                // Aunque se corte, lo que ya se ha visto forma parte de la conversación.
                if r.texto.is_empty() {
                    historial.pop();
                } else {
                    historial.push(json!({"role": "assistant", "content": r.texto}));
                }
                if !r.cortada {
                    let _ = writeln!(salida);
                }
            }
            Err(e) => {
                historial.pop(); // la pregunta no obtuvo respuesta: se puede repetir
                let _ = writeln!(salida, "\nError: {e}");
                if let ErrorCli::Api { codigo, .. } = &e
                    && codigo == "context_exceeded"
                {
                    let _ = writeln!(
                        salida,
                        "Escribe /nuevo para empezar una conversación nueva."
                    );
                }
            }
        }
    }
}

/// Envía la conversación y escribe la respuesta a medida que llega. Ctrl+C la corta.
fn responder<S: Write>(
    cliente: &Cliente,
    id: &str,
    mensajes: &[Value],
    salida: &mut S,
) -> Result<Respuesta, ErrorCli> {
    let cuerpo = json!({"model": id, "messages": mensajes, "stream": true});
    let respuesta = cliente.post_sin_leer("/api/chat", &cuerpo)?;
    let atenuar = std::io::stdout().is_terminal(); // el razonamiento, en gris (solo en una terminal)
    let mut texto = String::new();
    let mut pensando = false;
    for linea in BufReader::new(respuesta.into_body().into_reader()).lines() {
        if locall_system::ctrl_c_pulsado() {
            // Al salir de aquí se suelta la respuesta y se cierra la conexión: el motor deja de generar.
            let _ = writeln!(
                salida,
                "{}\n[respuesta cortada]",
                if pensando && atenuar { "\x1b[0m" } else { "" }
            );
            return Ok(Respuesta {
                texto,
                cortada: true,
            });
        }
        let linea = linea.map_err(|e| ErrorCli::Red(format!("la respuesta se cortó: {e}")))?;
        let Ok(evento) = serde_json::from_str::<Value>(&linea) else {
            continue;
        };
        match evento["type"].as_str() {
            Some("reasoning") => {
                if !pensando && atenuar {
                    let _ = write!(salida, "\x1b[2m");
                }
                pensando = true;
                let _ = write!(salida, "{}", evento["text"].as_str().unwrap_or(""));
            }
            Some("text") => {
                if pensando {
                    let _ = write!(salida, "{}\n\n", if atenuar { "\x1b[0m" } else { "" });
                    pensando = false;
                }
                let trozo = evento["text"].as_str().unwrap_or("");
                texto.push_str(trozo);
                let _ = write!(salida, "{trozo}");
            }
            Some("done") => break,
            Some("error") => {
                return Err(ErrorCli::Api {
                    estado: 0,
                    codigo: evento["error"]["code"].as_str().unwrap_or("").to_owned(),
                    mensaje: evento["error"]["message"]
                        .as_str()
                        .unwrap_or("error")
                        .to_owned(),
                });
            }
            _ => {}
        }
        let _ = salida.flush();
    }
    if pensando && atenuar {
        let _ = write!(salida, "\x1b[0m");
    }
    Ok(Respuesta {
        texto,
        cortada: false,
    })
}

#[cfg(test)]
mod pruebas {
    use std::cell::RefCell;

    use super::*;

    /// Simula un chat: `entrada` son las líneas que escribe el usuario; el "modelo" responde "R<n>".
    fn simular(entrada: &str) -> (String, Vec<Vec<Value>>) {
        let enviadas: RefCell<Vec<Vec<Value>>> = RefCell::new(Vec::new());
        let mut salida = Vec::new();
        conversar(
            entrada.as_bytes(),
            &mut salida,
            |mensajes, salida: &mut Vec<u8>| {
                let mut e = enviadas.borrow_mut();
                e.push(mensajes.to_vec());
                let texto = format!("R{}", e.len());
                let _ = write!(salida, "{texto}");
                Ok(Respuesta {
                    texto,
                    cortada: false,
                })
            },
            || false,
        )
        .unwrap();
        (String::from_utf8(salida).unwrap(), enviadas.into_inner())
    }

    #[test]
    fn recuerda_la_conversacion() {
        let (salida, enviadas) = simular("hola\n¿y tú?\n/salir\nesto ya no\n");
        assert_eq!(enviadas.len(), 2, "tras /salir no se envía nada más");
        // La segunda pregunta lleva toda la conversación anterior.
        assert_eq!(
            enviadas[1],
            vec![
                json!({"role": "user", "content": "hola"}),
                json!({"role": "assistant", "content": "R1"}),
                json!({"role": "user", "content": "¿y tú?"}),
            ]
        );
        assert!(salida.contains("R1") && salida.contains("R2"));
    }

    #[test]
    fn nuevo_olvida_lo_anterior_y_las_lineas_vacias_no_cuentan() {
        let (salida, enviadas) = simular("hola\n/nuevo\n\n   \notra\n");
        assert_eq!(enviadas.len(), 2);
        assert_eq!(
            enviadas[1],
            vec![json!({"role": "user", "content": "otra"})]
        );
        assert!(salida.contains("Conversación nueva."));
    }

    #[test]
    fn ordenes_del_chat() {
        let (salida, enviadas) = simular("/ayuda\n/rara\n");
        assert!(enviadas.is_empty());
        assert!(salida.contains("/nuevo") && salida.contains("Orden desconocida «/rara»"));
    }

    #[test]
    fn un_error_no_cierra_el_chat_y_la_pregunta_se_puede_repetir() {
        let mut intentos = 0;
        let mut salida = Vec::new();
        conversar(
            "muy larga\notra\n".as_bytes(),
            &mut salida,
            |mensajes, _: &mut Vec<u8>| {
                intentos += 1;
                if intentos == 1 {
                    return Err(ErrorCli::Api {
                        estado: 400,
                        codigo: "context_exceeded".into(),
                        mensaje: "no cabe".into(),
                    });
                }
                // La pregunta que falló no se queda en la conversación.
                assert_eq!(mensajes, [json!({"role": "user", "content": "otra"})]);
                Ok(Respuesta {
                    texto: "vale".into(),
                    cortada: false,
                })
            },
            || false,
        )
        .unwrap();
        let salida = String::from_utf8(salida).unwrap();
        assert!(
            salida.contains("Error: no cabe") && salida.contains("/nuevo"),
            "{salida}"
        );
        assert_eq!(intentos, 2);
    }

    #[test]
    fn ctrl_c_esperando_sale() {
        let mut salida = Vec::new();
        conversar(
            "hola\n".as_bytes(),
            &mut salida,
            |_, _: &mut Vec<u8>| panic!("no debería preguntar al modelo"),
            || true,
        )
        .unwrap();
    }
}
