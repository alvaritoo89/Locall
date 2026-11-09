//! `locall pull`, `import` y `rm` **de principio a fin**: el `locall.exe` real, un Hugging Face falso y una carpeta
//! de datos y un puerto temporales.

// El Hugging Face falso de las pruebas de la API (el mismo archivo, sin copiarlo).
#[path = "../../locall-api/tests/comun/hf.rs"]
mod hf;

use std::path::PathBuf;
use std::process::{Command, Stdio};

use hf::{Archivo, HfFalso, Repo, gguf, hf_falso};

struct Entorno {
    datos: PathBuf,
    hf: HfFalso,
}

impl Entorno {
    fn nuevo(nombre: &str, repos: Vec<Repo>) -> Entorno {
        let datos =
            std::env::temp_dir().join(format!("locall-clim-{nombre}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&datos);
        std::fs::create_dir_all(&datos).unwrap();
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
        Entorno {
            datos,
            hf: hf_falso(repos),
        }
    }

    /// `locall <argumentos>` (sin terminal en la entrada); devuelve (código, salida, errores).
    fn locall(&self, argumentos: &[&str]) -> (i32, String, String) {
        let salida = Command::new(env!("CARGO_BIN_EXE_locall"))
            .args(argumentos)
            .current_dir(&self.datos)
            .env("LOCALL_DATOS", &self.datos)
            .env("LOCALL_HUGGING_FACE", &self.hf.base)
            .env("LOCALL_MOTOR", r"C:\Locall-poc\motor\cpu\llama-server.exe")
            .stdin(Stdio::null())
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

fn repo(nombre: &str, archivos: Vec<(&str, Vec<u8>)>) -> Repo {
    Repo {
        nombre: nombre.into(),
        archivos: archivos
            .into_iter()
            .map(|(ruta, datos)| Archivo {
                ruta: ruta.into(),
                datos,
                con_huella: true,
            })
            .collect(),
        licencia: Some("mit".into()),
        protegido: false,
        lento: false,
    }
}

fn llama(relleno: usize) -> Vec<u8> {
    gguf(&[("general.architecture", "llama")], &[], relleno)
}

#[test]
fn pull_lista_e_instala() {
    let e = Entorno::nuevo(
        "pull",
        vec![
            repo(
                "prueba/modelos",
                vec![
                    ("Pequeno-Q4_K_M.gguf", llama(2_000_000)),
                    ("Pequeno-Q8_0.gguf", llama(10)),
                ],
            ),
            repo(
                "prueba/raro",
                vec![(
                    "raro.gguf",
                    gguf(&[("general.architecture", "inventada")], &[], 10),
                )],
            ),
        ],
    );

    // Sin archivo: lista los GGUF y dice cómo instalar uno.
    let (codigo, salida, errores) = e.locall(&["pull", "prueba/modelos"]);
    assert_eq!(codigo, 0, "{errores}");
    assert!(
        salida.contains("Pequeno-Q4_K_M.gguf") && salida.contains("2.0 MB"),
        "{salida}"
    );
    assert!(salida.contains("licencia mit"), "{salida}");
    assert!(
        salida.contains("locall pull prueba/modelos Pequeno-Q4_K_M.gguf"),
        "{salida}"
    );

    // Con archivo: instala, con progreso en stderr (sin terminal: una línea cada 10 %) y el resultado en stdout.
    let (codigo, salida, errores) = e.locall(&["pull", "prueba/modelos", "Pequeno-Q4_K_M.gguf"]);
    assert_eq!(codigo, 0, "{errores}");
    assert!(salida.contains("Instalado «pequeno-q4_k_m»"), "{salida}");
    assert!(errores.contains("100 %"), "{errores}");
    assert!(
        !errores.contains('\r'),
        "sin terminal no se redibuja la línea"
    );
    let (_, lista, _) = e.locall(&["list"]);
    assert!(lista.contains("pequeno-q4_k_m"), "{lista}");

    // Con --id, otro nombre.
    let (codigo, salida, _) =
        e.locall(&["pull", "prueba/modelos", "Pequeno-Q8_0.gguf", "--id", "q8"]);
    assert_eq!(codigo, 0);
    assert!(salida.contains("Instalado «q8»"), "{salida}");

    // Errores claros, código 1.
    let casos = [
        (
            vec!["pull", "prueba/modelos", "Pequeno-Q4_K_M.gguf"],
            "ya hay un modelo instalado",
        ),
        (vec!["pull", "prueba/raro", "raro.gguf"], "«inventada»"),
        (vec!["pull", "prueba/no-existe"], "no tiene el repositorio"),
        (
            vec!["pull", "prueba/modelos", "otro.gguf"],
            "no tiene «otro.gguf»",
        ),
    ];
    for (argumentos, fragmento) in casos {
        let (codigo, _, errores) = e.locall(&argumentos);
        assert_eq!(codigo, 1, "{argumentos:?}");
        assert!(errores.contains(fragmento), "{argumentos:?}: {errores}");
    }
}

#[test]
fn import_y_rm() {
    let e = Entorno::nuevo("importar", vec![]);
    std::fs::write(e.datos.join("Local-Q8_0.gguf"), llama(5_000)).unwrap();

    // Ruta relativa a la carpeta actual.
    let (codigo, salida, errores) = e.locall(&["import", "Local-Q8_0.gguf"]);
    assert_eq!(codigo, 0, "{errores}");
    assert!(salida.contains("Importado «local-q8_0»"), "{salida}");

    // Sin terminal y sin --yes, no borra: pide --yes (código 2).
    let (codigo, _, errores) = e.locall(&["rm", "local-q8_0"]);
    assert_eq!(codigo, 2);
    assert!(errores.contains("añade --yes"), "{errores}");
    let (_, lista, _) = e.locall(&["list"]);
    assert!(lista.contains("local-q8_0"), "sigue instalado: {lista}");

    let (codigo, salida, _) = e.locall(&["rm", "local-q8_0", "--yes"]);
    assert_eq!(codigo, 0);
    assert!(salida.contains("Eliminado «local-q8_0»"), "{salida}");
    assert!(
        e.datos.join("Local-Q8_0.gguf").is_file(),
        "el original del usuario no se toca"
    );

    let (codigo, _, errores) = e.locall(&["rm", "local-q8_0", "-y"]);
    assert_eq!(codigo, 1);
    assert!(
        errores.contains("no hay ningún modelo instalado como «local-q8_0»"),
        "{errores}"
    );
}
