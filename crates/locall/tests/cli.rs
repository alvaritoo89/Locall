//! The command line **end to end**: the real `locall.exe` runs, with a temporary data folder and port (it does not
//! touch the user's Locall).

use std::path::PathBuf;
use std::process::Command;

use locall_config::Rutas;
use serde_json::json;

/// A temporary data folder. When done, it shuts down any Locall that was started and deletes the folder.
struct Entorno {
    datos: PathBuf,
    puerto: u16,
}

impl Entorno {
    fn nuevo(nombre: &str) -> Entorno {
        let datos =
            std::env::temp_dir().join(format!("locall-cli-{nombre}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&datos);
        std::fs::create_dir_all(&datos).unwrap();
        // A free port (and, in case a test fails halfway, automatic shutdown after 1 minute without use).
        let puerto = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        std::fs::write(
            datos.join("config.toml"),
            format!("puerto = {puerto}\ncierre_inactividad_minutos = 1\n"),
        )
        .unwrap();
        Entorno { datos, puerto }
    }

    /// Runs `locall <arguments>`; returns (exit code, stdout, stderr).
    fn locall(&self, argumentos: &[&str]) -> (i32, String, String) {
        let salida = Command::new(env!("CARGO_BIN_EXE_locall"))
            .args(argumentos)
            .env("LOCALL_DATOS", &self.datos)
            .env("LOCALL_MOTOR", r"C:\Locall-poc\motor\cpu\llama-server.exe")
            .output()
            .unwrap();
        (
            salida.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&salida.stdout).into_owned(),
            String::from_utf8_lossy(&salida.stderr).into_owned(),
        )
    }
}

impl Drop for Entorno {
    fn drop(&mut self) {
        let _ = self.locall(&["stop"]);
        let _ = std::fs::remove_dir_all(&self.datos);
    }
}

#[test]
fn ayuda_version_y_errores_de_uso() {
    let e = Entorno::nuevo("ayuda");
    let (codigo, salida, _) = e.locall(&["help"]);
    assert_eq!(codigo, 0);
    assert!(salida.contains("Modelos:") && salida.contains("locall <orden>"));
    let (codigo, salida, _) = e.locall(&["--version"]);
    assert_eq!(
        (codigo, salida.trim()),
        (0, concat!("locall ", env!("CARGO_PKG_VERSION")))
    );

    let (codigo, _, errores) = e.locall(&["lanzar"]);
    assert_eq!(codigo, 2, "los errores de uso salen con el código 2");
    assert!(errores.contains("orden desconocida «lanzar»"), "{errores}");
    assert!(errores.contains("locall help"));
}

#[test]
fn abre_locall_cuando_hace_falta_y_lo_cierra_con_stop() {
    let e = Entorno::nuevo("ciclo");

    // 1) Not running: status says so and does NOT start it.
    let (codigo, salida, _) = e.locall(&["status"]);
    assert_eq!(codigo, 0);
    assert!(salida.contains("no está abierto"), "{salida}");

    // 2) list starts it in the background.
    let (codigo, salida, errores) = e.locall(&["list"]);
    assert_eq!(codigo, 0, "{errores}");
    assert!(salida.contains("No hay modelos instalados"), "{salida}");
    // It says it is starting it, and with which folder (on stderr: normal output stays clean).
    assert!(
        errores.contains("Abriendo Locall en segundo plano"),
        "{errores}"
    );
    assert!(
        errores.contains(&e.datos.display().to_string()),
        "{errores}"
    );
    let (_, _, errores) = e.locall(&["list"]);
    assert!(
        !errores.contains("Abriendo"),
        "ya estaba abierto: {errores}"
    );

    // 3) Now status sees it running, on our port.
    let (_, salida, _) = e.locall(&["status"]);
    assert!(
        salida.contains(&format!("http://127.0.0.1:{}", e.puerto)),
        "{salida}"
    );
    assert!(salida.contains("Ningún modelo cargado"), "{salida}");

    // 4) With a model installed (through the API), list and show display it.
    let token = std::fs::read_to_string(Rutas::en(e.datos.clone()).token()).unwrap();
    let cliente =
        locall_cli::Cliente::nuevo(format!("http://127.0.0.1:{}", e.puerto), token.trim());
    let mut gguf = b"GGUF".to_vec();
    gguf.extend(3u32.to_le_bytes());
    gguf.extend(0u64.to_le_bytes());
    gguf.extend(1u64.to_le_bytes());
    let clave = "general.architecture";
    gguf.extend((clave.len() as u64).to_le_bytes());
    gguf.extend(clave.as_bytes());
    gguf.extend(8u32.to_le_bytes());
    gguf.extend(5u64.to_le_bytes());
    gguf.extend(b"llama");
    let archivo = e.datos.join("Mini-Q4_0.gguf");
    std::fs::write(&archivo, gguf).unwrap();
    cliente
        .post(
            "/api/models/import",
            &json!({"path": archivo.to_string_lossy(), "id": "mini"}),
        )
        .unwrap();

    let (_, salida, _) = e.locall(&["list"]);
    assert!(salida.starts_with("MODELO"), "{salida}");
    assert!(
        salida.contains("mini") && salida.contains("Q4_0") && salida.contains("nunca"),
        "{salida}"
    );
    let (_, salida, _) = e.locall(&["show", "mini"]);
    assert!(salida.contains("Arquitectura   llama"), "{salida}");
    assert!(salida.contains("Mini-Q4_0.gguf"), "{salida}");

    // 5) A model that does not exist: a clear error and exit code 1.
    let (codigo, _, errores) = e.locall(&["show", "no-esta"]);
    assert_eq!(codigo, 1);
    assert!(
        errores.contains("no hay ningún modelo instalado como «no-esta»"),
        "{errores}"
    );

    // 6) stop really shuts it down.
    let (codigo, salida, _) = e.locall(&["stop"]);
    assert_eq!((codigo, salida.trim()), (0, "Locall cerrado."));
    let (_, salida, _) = e.locall(&["status"]);
    assert!(salida.contains("no está abierto"), "{salida}");
    let (_, salida, _) = e.locall(&["stop"]);
    assert_eq!(salida.trim(), "Locall no estaba abierto.");
}
