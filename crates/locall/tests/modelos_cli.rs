//! `locall pull`, `import` and `rm` **end to end**: the real `locall.exe`, a fake Hugging Face and a temporary data
//! folder and port.

// The fake Hugging Face from the API tests (the same file, not a copy).
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

    /// `locall <arguments>` (no terminal on standard input); returns (exit code, stdout, stderr).
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

    // Without a file: lists the GGUFs and says how to install one.
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

    // With a file: installs, with progress on stderr (no terminal: one line every 10%) and the result on stdout.
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

    // With --id, another name.
    let (codigo, salida, _) =
        e.locall(&["pull", "prueba/modelos", "Pequeno-Q8_0.gguf", "--id", "q8"]);
    assert_eq!(codigo, 0);
    assert!(salida.contains("Instalado «q8»"), "{salida}");

    // Clear errors, exit code 1.
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

    // A path relative to the current folder.
    let (codigo, salida, errores) = e.locall(&["import", "Local-Q8_0.gguf"]);
    assert_eq!(codigo, 0, "{errores}");
    assert!(salida.contains("Importado «local-q8_0»"), "{salida}");

    // Without a terminal and without --yes, nothing is deleted: it asks for --yes (exit code 2).
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
