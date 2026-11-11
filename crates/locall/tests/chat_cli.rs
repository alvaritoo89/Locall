//! `locall run` **end to end** with the real engine: the Phase 0 model is imported with the CLI and asked a question.
//! Skipped if the engine or the model is missing.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const MOTOR: &str = r"C:\Locall-poc\motor\cpu\llama-server.exe";
const MODELO: &str = r"C:\Locall-poc\modelos\qwen2.5-1.5b-instruct-q4_k_m.gguf";

struct Entorno {
    datos: PathBuf,
}

impl Entorno {
    fn nuevo(nombre: &str) -> Option<Entorno> {
        if !(PathBuf::from(MOTOR).is_file() && PathBuf::from(MODELO).is_file()) {
            eprintln!("(prueba saltada: faltan el motor o el modelo)");
            return None;
        }
        let datos =
            std::env::temp_dir().join(format!("locall-clic-{nombre}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&datos);
        std::fs::create_dir_all(&datos).unwrap();
        let puerto = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        std::fs::write(
            datos.join("config.toml"),
            format!("puerto = {puerto}\ncierre_inactividad_minutos = 1\ncontexto = 2048\n"),
        )
        .unwrap();
        Some(Entorno { datos })
    }

    /// `locall <arguments>` with `entrada` on standard input; returns (exit code, stdout, stderr).
    fn locall(&self, argumentos: &[&str], entrada: &str) -> (i32, String, String) {
        let mut hijo = Command::new(env!("CARGO_BIN_EXE_locall"))
            .args(argumentos)
            .env("LOCALL_DATOS", &self.datos)
            .env("LOCALL_MOTOR", MOTOR)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        hijo.stdin
            .take()
            .unwrap()
            .write_all(entrada.as_bytes())
            .unwrap();
        let salida = hijo.wait_with_output().unwrap();
        (
            salida.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&salida.stdout).into_owned(),
            String::from_utf8_lossy(&salida.stderr).into_owned(),
        )
    }
}

impl Drop for Entorno {
    fn drop(&mut self) {
        let _ = self.locall(&["stop"], "");
        let _ = std::fs::remove_dir_all(&self.datos);
    }
}

#[test]
fn importa_y_responde_con_el_motor_real() {
    let Some(e) = Entorno::nuevo("run") else {
        return;
    };
    let (codigo, salida, errores) = e.locall(&["import", MODELO, "--id", "qwen"], "");
    assert_eq!(codigo, 0, "{errores}");
    assert!(salida.contains("Pruébalo: locall run qwen"), "{salida}");

    // The question as arguments (no quotes: the words are joined).
    let (codigo, salida, errores) = e.locall(
        &[
            "run",
            "qwen",
            "¿Capital",
            "de",
            "Francia?",
            "Una",
            "palabra.",
        ],
        "",
    );
    eprintln!("respuesta: {salida:?}");
    assert_eq!(codigo, 0, "{errores}");
    // The CLI sets neither temperature nor seed (the model answers with some randomness, as in a normal chat), so this
    // does not check WHAT it says, only that it answers. That answers are correct is tested by the API tests with
    // temperature 0.
    assert!(salida.trim().len() > 1, "respuesta vacía: {salida:?}");
    assert!(
        errores.contains("Cargando el modelo"),
        "la primera vez avisa: {errores}"
    );

    // The question on standard input (as in `echo … | locall run qwen`). The model is already loaded: no notice.
    let (codigo, salida, errores) =
        e.locall(&["run", "qwen"], "¿Capital de Italia? Una palabra.\n");
    eprintln!("respuesta: {salida:?}");
    assert_eq!(codigo, 0, "{errores}");
    assert!(salida.trim().len() > 1, "respuesta vacía: {salida:?}");
    assert!(!errores.contains("Cargando"), "{errores}");

    // Clear errors.
    let (codigo, _, errores) = e.locall(&["run", "no-esta", "hola"], "");
    assert_eq!(codigo, 1);
    assert!(
        errores.contains("no hay ningún modelo instalado como «no-esta»"),
        "{errores}"
    );
    let (codigo, _, errores) = e.locall(&["run", "qwen"], "   \n");
    assert_eq!(codigo, 2);
    assert!(errores.contains("la pregunta está vacía"), "{errores}");
}
