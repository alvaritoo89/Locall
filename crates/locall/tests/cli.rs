//! La línea de comandos **de principio a fin**: se ejecuta el `locall.exe` de verdad, con una carpeta de datos y un
//! puerto temporales (no toca el Locall del usuario).

use std::path::PathBuf;
use std::process::Command;

use locall_config::Rutas;
use serde_json::json;

/// Una carpeta de datos temporal. Al terminar, cierra el Locall que se haya abierto y la borra.
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
        // Un puerto libre (y, por si una prueba fallara a medias, cierre automático tras 1 minuto sin uso).
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

    /// Ejecuta `locall <argumentos>`; devuelve (código de salida, salida, errores).
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
    assert!(salida.contains("Órdenes:") && salida.contains("locall <orden>"));
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

    // 1) Sin abrir: status lo dice y NO lo abre.
    let (codigo, salida, _) = e.locall(&["status"]);
    assert_eq!(codigo, 0);
    assert!(salida.contains("no está abierto"), "{salida}");

    // 2) list lo abre en segundo plano.
    let (codigo, salida, errores) = e.locall(&["list"]);
    assert_eq!(codigo, 0, "{errores}");
    assert!(salida.contains("No hay modelos instalados"), "{salida}");

    // 3) Ahora status lo ve abierto, en nuestro puerto.
    let (_, salida, _) = e.locall(&["status"]);
    assert!(
        salida.contains(&format!("http://127.0.0.1:{}", e.puerto)),
        "{salida}"
    );
    assert!(salida.contains("Ningún modelo cargado"), "{salida}");

    // 4) Con un modelo instalado (por la API), list y show lo muestran.
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

    // 5) Un modelo que no existe: error claro y código 1.
    let (codigo, _, errores) = e.locall(&["show", "no-esta"]);
    assert_eq!(codigo, 1);
    assert!(
        errores.contains("no hay ningún modelo instalado como «no-esta»"),
        "{errores}"
    );

    // 6) stop lo cierra de verdad.
    let (codigo, salida, _) = e.locall(&["stop"]);
    assert_eq!((codigo, salida.trim()), (0, "Locall cerrado."));
    let (_, salida, _) = e.locall(&["status"]);
    assert!(salida.contains("no está abierto"), "{salida}");
    let (_, salida, _) = e.locall(&["stop"]);
    assert_eq!(salida.trim(), "Locall no estaba abierto.");
}
