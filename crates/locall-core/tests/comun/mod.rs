//! A **fake Hugging Face** (127.0.0.1) to test installation without internet, and common helpers.

#![allow(dead_code)] // each test file uses only part of it

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

use locall_config::Rutas;
use locall_core::{Nucleo, OpcionesNucleo};

pub const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

/// A file of the fake repository.
#[derive(Clone)]
pub struct Archivo {
    pub ruta: String,
    pub datos: Vec<u8>,
    /// `false` = like a small git file, without sha256 (LFS).
    pub con_huella: bool,
}

/// A fake repository.
#[derive(Clone)]
pub struct Repo {
    pub nombre: String,
    pub archivos: Vec<Archivo>,
    pub licencia: Option<String>,
    pub protegido: bool,
}

/// The fake server, with the list of requested routes (to check what was downloaded).
pub struct HfFalso {
    pub base: String,
    pub pedidas: Arc<Mutex<Vec<String>>>,
    /// Extra standalone files it serves (exact path → content), for example a catalog and its signature. They can be
    /// changed during the test.
    pub extra: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
}

pub fn hf_falso(repos: Vec<Repo>) -> HfFalso {
    let escucha = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", escucha.local_addr().unwrap());
    let pedidas = Arc::new(Mutex::new(Vec::new()));
    let registro = Arc::clone(&pedidas);
    let repos: Arc<BTreeMap<String, Repo>> =
        Arc::new(repos.into_iter().map(|r| (r.nombre.clone(), r)).collect());
    let extra: Arc<Mutex<BTreeMap<String, Vec<u8>>>> = Arc::default();
    let extra_servidor = Arc::clone(&extra);
    thread::spawn(move || {
        for conexion in escucha.incoming() {
            let Ok(conexion) = conexion else { continue };
            let repos = Arc::clone(&repos);
            let registro = Arc::clone(&registro);
            let extra = Arc::clone(&extra_servidor);
            thread::spawn(move || atender(conexion, &repos, &registro, &extra));
        }
    });
    HfFalso {
        base,
        pedidas,
        extra,
    }
}

fn atender(
    mut c: TcpStream,
    repos: &BTreeMap<String, Repo>,
    pedidas: &Mutex<Vec<String>>,
    extra: &Mutex<BTreeMap<String, Vec<u8>>>,
) {
    let (ruta, desde) = {
        let mut lector = BufReader::new(&c);
        let mut primera = String::new();
        let _ = lector.read_line(&mut primera);
        let ruta = primera.split_whitespace().nth(1).unwrap_or("/").to_owned();
        let mut desde = None;
        loop {
            let mut linea = String::new();
            if lector.read_line(&mut linea).unwrap_or(0) == 0 || linea == "\r\n" {
                break;
            }
            if let Some(v) = linea.to_ascii_lowercase().strip_prefix("range: bytes=") {
                desde = v.trim().trim_end_matches('-').parse::<usize>().ok();
            }
        }
        (ruta, desde)
    };
    pedidas.lock().unwrap().push(ruta.clone());
    let suelto = extra.lock().unwrap().get(&ruta).cloned();
    if let Some(datos) = suelto {
        let cabecera = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            datos.len()
        );
        let _ = c.write_all(cabecera.as_bytes());
        let _ = c.write_all(&datos);
        return;
    }

    let responder = |c: &mut TcpStream, estado: &str, cuerpo: &[u8], extra: &str| {
        let cabecera = format!(
            "HTTP/1.1 {estado}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n",
            cuerpo.len()
        );
        let _ = c.write_all(cabecera.as_bytes());
        let _ = c.write_all(cuerpo);
    };
    let no_existe = |c: &mut TcpStream| responder(c, "401 Unauthorized", b"{}", "");

    // /api/models/{owner}/{name}/revision/{rev}
    // /api/models/{owner}/{name}/tree/{commit}?recursive=true
    // /{owner}/{name}/resolve/{commit}/{path}
    let partes: Vec<&str> = ruta.trim_start_matches('/').splitn(6, '/').collect();
    if partes.first() == Some(&"api") && partes.len() >= 5 {
        let Some(repo) = repos.get(&format!("{}/{}", partes[2], partes[3])) else {
            return no_existe(&mut c);
        };
        match partes[4] {
            "revision" => {
                let etiquetas: Vec<String> = repo
                    .licencia
                    .iter()
                    .map(|l| format!("\"license:{l}\""))
                    .chain(["\"gguf\"".to_owned()])
                    .collect();
                let gated = if repo.protegido {
                    "\"manual\""
                } else {
                    "false"
                };
                let json = format!(
                    "{{\"sha\":\"{COMMIT}\",\"tags\":[{}],\"gated\":{gated}}}",
                    etiquetas.join(",")
                );
                responder(&mut c, "200 OK", json.as_bytes(), "");
            }
            "tree" => {
                let entradas: Vec<String> = repo
                    .archivos
                    .iter()
                    .map(|a| {
                        let lfs = if a.con_huella {
                            format!(
                                ",\"lfs\":{{\"oid\":\"{}\",\"size\":{}}}",
                                locall_store::huella_de_bytes(&a.datos).hex(),
                                a.datos.len()
                            )
                        } else {
                            String::new()
                        };
                        format!(
                            "{{\"type\":\"file\",\"oid\":\"x\",\"size\":{},\"path\":\"{}\"{lfs}}}",
                            a.datos.len(),
                            a.ruta
                        )
                    })
                    .collect();
                let json = format!("[{}]", entradas.join(","));
                responder(&mut c, "200 OK", json.as_bytes(), "");
            }
            _ => no_existe(&mut c),
        }
        return;
    }
    // In the download, everything after the commit is the file path (it may contain folders).
    let partes: Vec<&str> = ruta.trim_start_matches('/').splitn(5, '/').collect();
    if partes.len() == 5 && partes[2] == "resolve" {
        let Some(repo) = repos.get(&format!("{}/{}", partes[0], partes[1])) else {
            return no_existe(&mut c);
        };
        let ruta_archivo = partes[4].replace("%20", " ");
        let Some(archivo) = repo.archivos.iter().find(|a| a.ruta == ruta_archivo) else {
            return responder(&mut c, "404 Not Found", b"", "");
        };
        match desde {
            Some(d) => {
                let rango = format!(
                    "Content-Range: bytes {d}-{}/{}\r\n",
                    archivo.datos.len() - 1,
                    archivo.datos.len()
                );
                responder(&mut c, "206 Partial Content", &archivo.datos[d..], &rango);
            }
            None => responder(&mut c, "200 OK", &archivo.datos, ""),
        }
        return;
    }
    no_existe(&mut c);
}

/// A minimal GGUF v3 (no tensors) with these metadata, plus `relleno` bytes at the end (the "weights").
pub fn gguf(textos: &[(&str, &str)], numeros_u16: &[(&str, u16)], relleno: usize) -> Vec<u8> {
    let mut v = b"GGUF".to_vec();
    v.extend(3u32.to_le_bytes());
    v.extend(0u64.to_le_bytes());
    v.extend(((textos.len() + numeros_u16.len()) as u64).to_le_bytes());
    let cadena = |v: &mut Vec<u8>, s: &str| {
        v.extend((s.len() as u64).to_le_bytes());
        v.extend(s.as_bytes());
    };
    for (clave, valor) in textos {
        cadena(&mut v, clave);
        v.extend(8u32.to_le_bytes());
        cadena(&mut v, valor);
    }
    for (clave, valor) in numeros_u16 {
        cadena(&mut v, clave);
        v.extend(2u32.to_le_bytes());
        v.extend(valor.to_le_bytes());
    }
    v.extend((0..relleno).map(|i| (i % 251) as u8));
    v
}

/// A temporary data folder deleted at the end.
pub struct CarpetaDePrueba(pub PathBuf);

impl CarpetaDePrueba {
    pub fn nueva(nombre: &str) -> CarpetaDePrueba {
        let ruta =
            std::env::temp_dir().join(format!("locall-core-{nombre}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ruta);
        CarpetaDePrueba(ruta)
    }
}

impl Drop for CarpetaDePrueba {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The test public key the catalogs in `tests/datos/` are signed with.
pub fn clave_de_prueba() -> String {
    include_str!("../datos/prueba.pub")
        .lines()
        .nth(1)
        .unwrap()
        .trim()
        .to_owned()
}

/// Opens a core that looks for the catalog in `hf` (at `/catalogo/catalogo.json`) and checks it with the test key.
pub fn nucleo_con_catalogo(carpeta: &CarpetaDePrueba, hf: &HfFalso) -> Nucleo {
    let mut opciones = OpcionesNucleo::nuevas(
        Rutas::en(carpeta.0.clone()),
        r"C:\Locall-poc\motor\cpu\llama-server.exe",
    );
    opciones.hugging_face = hf.base.clone();
    opciones.catalogo = format!("{}/catalogo/catalogo.json", hf.base);
    opciones.clave_catalogo = clave_de_prueba();
    Nucleo::abrir(opciones).unwrap()
}

/// Serves this catalog and its signature on `hf` (files from `tests/datos/`).
pub fn servir_catalogo(hf: &HfFalso, catalogo: &[u8], firma: &[u8]) {
    let mut extra = hf.extra.lock().unwrap();
    extra.insert("/catalogo/catalogo.json".into(), catalogo.to_vec());
    extra.insert("/catalogo/catalogo.json.minisig".into(), firma.to_vec());
}

/// Opens a core in `carpeta`, pointing at `hf` (or at the real Hugging Face if it is `None`).
pub fn nucleo(carpeta: &CarpetaDePrueba, hf: Option<&HfFalso>) -> Nucleo {
    let mut opciones = OpcionesNucleo::nuevas(
        Rutas::en(carpeta.0.clone()),
        r"C:\Locall-poc\motor\cpu\llama-server.exe",
    );
    if let Some(hf) = hf {
        opciones.hugging_face = hf.base.clone();
    }
    Nucleo::abrir(opciones).unwrap()
}
