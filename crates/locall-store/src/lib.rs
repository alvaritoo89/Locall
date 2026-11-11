//! # locall-store
//!
//! Locall's **model store**.
//!
//! Each model file is stored **only once**, named after its sha256 hash, and only after checking that its content
//! matches the expected hash:
//!
//! ```text
//! <root>\
//! ├── blobs\sha256-<64 hexadecimal characters>   ← the content, verified
//! ├── snapshots\<model>\<revision>\<original name>   ← hard links to the blobs (0 extra bytes)
//! └── tmp\                                          ← half-saved files
//! ```
//!
//! Saving is **atomic**: the file is written to `tmp\`, verified and only then moved to `blobs\`.
//! If something fails halfway, an incomplete file is never left in `blobs\`.
//!
//! **Snapshot folders** exist because llama.cpp finds the parts of a split model by their name
//! (`…-00001-of-00002.gguf`): there each file appears with its original name, as a hard link to the blob.

mod mantenimiento;
mod nombres;
mod sistema;
mod snapshot;

pub use mantenimiento::{BlobSinUsar, MARGEN_ESPACIO};
pub use nombres::{NombreArchivo, Segmento};
pub use snapshot::Snapshot;

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

/// Size of the chunks used to read and copy (1 MiB).
const TROZO: usize = 1024 * 1024;

// ─── Hash ─────────────────────────────────────────────────────────────────────

/// A **valid** sha256 hash: exactly 64 lowercase hexadecimal characters.
///
/// It is a *newtype*: a type of our own wrapping a `String`. Since the inner `String` is private, the only way to get
/// a `Huella` is with [`Huella::desde_hex`] (which validates) or by computing it. That makes it **impossible** for a
/// hash to contain `..`, `\` or other characters that could build a path outside the store.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Huella(String);

impl Huella {
    /// Creates a hash from text. Accepts the `sha256:` prefix and upper case (converted to lower case).
    pub fn desde_hex(texto: &str) -> Result<Huella, ErrorAlmacen> {
        let hex = texto.strip_prefix("sha256:").unwrap_or(texto);
        if hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            Ok(Huella(hex.to_ascii_lowercase()))
        } else {
            Err(ErrorAlmacen::HuellaInvalida(texto.to_string()))
        }
    }

    /// The 64 hexadecimal characters.
    pub fn hex(&self) -> &str {
        &self.0
    }

    /// The file name in `blobs\`: `sha256-<hex>`.
    pub fn nombre_blob(&self) -> String {
        format!("sha256-{}", self.0)
    }

    /// Builds the hash from the sha256 result (32 bytes).
    fn desde_bytes(bytes: &[u8]) -> Huella {
        Huella(bytes.iter().map(|b| format!("{b:02x}")).collect())
    }

    /// Builds the hash from a sha256 result computed outside the store (for example, by the downloader while it
    /// downloads). Requires exactly 32 bytes.
    pub fn desde_sha256(resultado: &[u8]) -> Result<Huella, ErrorAlmacen> {
        if resultado.len() == 32 {
            Ok(Huella::desde_bytes(resultado))
        } else {
            Err(ErrorAlmacen::HuellaInvalida(format!(
                "{} bytes (se esperaban 32)",
                resultado.len()
            )))
        }
    }
}

impl fmt::Display for Huella {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sha256:{}", self.0)
    }
}

// ─── Errors ───────────────────────────────────────────────────────────────────

/// What can go wrong in the store.
#[derive(Debug)]
pub enum ErrorAlmacen {
    /// File system error, with a description of what was being attempted.
    Io { contexto: String, error: io::Error },
    /// The text is not a valid sha256 hash.
    HuellaInvalida(String),
    /// The content does not match the expected hash: the file is damaged or is not the one it should be.
    HuellaNoCoincide { esperada: Huella, obtenida: Huella },
    /// A file or folder name is not safe (paths, forbidden characters, reserved names…).
    NombreInvalido {
        nombre: String,
        motivo: &'static str,
    },
    /// A file that is not in the store was requested.
    BlobNoEncontrado(Huella),
    /// A snapshot folder with no files.
    SnapshotVacio,
    /// Not enough disk space (counting the safety margin).
    EspacioInsuficiente { necesarios: u64, disponibles: u64 },
    /// An attempt was made to delete a file some model still uses.
    BlobEnUso(Huella),
}

impl fmt::Display for ErrorAlmacen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorAlmacen::Io { contexto, error } => write!(f, "{contexto}: {error}"),
            ErrorAlmacen::HuellaInvalida(t) => write!(f, "«{t}» no es una huella sha256 válida"),
            ErrorAlmacen::HuellaNoCoincide { esperada, obtenida } => write!(
                f,
                "el archivo no coincide con el original (esperada {esperada}, obtenida {obtenida}); se ha descartado"
            ),
            ErrorAlmacen::NombreInvalido { nombre, motivo } => {
                write!(f, "nombre no válido «{nombre}»: {motivo}")
            }
            ErrorAlmacen::BlobNoEncontrado(h) => write!(f, "el archivo {h} no está en el almacén"),
            ErrorAlmacen::SnapshotVacio => write!(f, "un modelo necesita al menos un archivo"),
            ErrorAlmacen::EspacioInsuficiente {
                necesarios,
                disponibles,
            } => write!(
                f,
                "no hay espacio suficiente: hacen falta {:.1} GB (más 1 GB de margen) y quedan {:.1} GB",
                *necesarios as f64 / 1e9,
                *disponibles as f64 / 1e9
            ),
            ErrorAlmacen::BlobEnUso(h) => {
                write!(f, "el archivo {h} lo usa algún modelo y no se puede borrar")
            }
        }
    }
}

impl std::error::Error for ErrorAlmacen {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ErrorAlmacen::Io { error, .. } => Some(error),
            _ => None,
        }
    }
}

/// Adds context to I/O errors: `.contexto("abrir el archivo")`.
trait ConContexto<T> {
    fn contexto(self, que: impl FnOnce() -> String) -> Result<T, ErrorAlmacen>;
}

impl<T> ConContexto<T> for io::Result<T> {
    fn contexto(self, que: impl FnOnce() -> String) -> Result<T, ErrorAlmacen> {
        self.map_err(|error| ErrorAlmacen::Io {
            contexto: que(),
            error,
        })
    }
}

// ─── Store ────────────────────────────────────────────────────────────────────

/// The model store, in a root folder.
#[derive(Debug, Clone)]
pub struct Almacen {
    raiz: PathBuf,
    blobs: PathBuf,
    snapshots: PathBuf,
    tmp: PathBuf,
}

/// Counter to give temporary files unique names.
static CONTADOR_TMP: AtomicU64 = AtomicU64::new(0);

impl Almacen {
    /// Opens (and creates if missing) a store at `raiz`.
    pub fn abrir(raiz: impl Into<PathBuf>) -> Result<Almacen, ErrorAlmacen> {
        let raiz = raiz.into();
        let blobs = raiz.join("blobs");
        let snapshots = raiz.join("snapshots");
        let tmp = raiz.join("tmp");
        for carpeta in [&blobs, &snapshots, &tmp] {
            fs::create_dir_all(carpeta)
                .contexto(|| format!("crear la carpeta {}", carpeta.display()))?;
        }
        Ok(Almacen {
            raiz,
            blobs,
            snapshots,
            tmp,
        })
    }

    /// Brings into the store a file that is **already on this same disk**, without copying it: verifies its hash and
    /// creates a hard link in `blobs\`. Useful to import already downloaded models without using twice the space.
    ///
    /// Important: the blob and the original file become **the same file**. If someone modified the original, the blob
    /// would change too; that is why Locall should only adopt files that will not be touched.
    pub fn adoptar_archivo(
        &self,
        origen: &Path,
        esperada: Option<&Huella>,
    ) -> Result<Huella, ErrorAlmacen> {
        let huella = huella_de_archivo(origen)?;
        if let Some(esperada) = esperada
            && *esperada != huella
        {
            return Err(ErrorAlmacen::HuellaNoCoincide {
                esperada: esperada.clone(),
                obtenida: huella,
            });
        }
        let destino = self.ruta_blob(&huella);
        if !destino.is_file() {
            fs::hard_link(origen, &destino).contexto(|| {
                format!(
                    "enlazar {} en el almacén (¿está en otro disco?)",
                    origen.display()
                )
            })?;
        }
        Ok(huella)
    }

    /// The store's root folder.
    pub fn raiz(&self) -> &Path {
        &self.raiz
    }

    /// Where the file with this hash is (or would be).
    pub fn ruta_blob(&self, huella: &Huella) -> PathBuf {
        self.blobs.join(huella.nombre_blob())
    }

    /// Do we already have a file with this hash?
    pub fn contiene(&self, huella: &Huella) -> bool {
        self.ruta_blob(huella).is_file()
    }

    /// Where a half-finished download of the file with this hash is kept: `tmp\sha256-<hex>.parcial`.
    /// Using the hash as the name, an interrupted download resumes by itself the next time.
    pub fn ruta_parcial(&self, huella: &Huella) -> PathBuf {
        self.tmp.join(format!("{}.parcial", huella.nombre_blob()))
    }

    /// Moves into `blobs\` a download that is already complete **and verified by whoever downloaded it**. It is instant
    /// (a move within the same disk) and atomic. If the blob already existed, the downloaded copy is discarded.
    pub fn confirmar_parcial(&self, huella: &Huella) -> Result<(), ErrorAlmacen> {
        let origen = self.ruta_parcial(huella);
        let destino = self.ruta_blob(huella);
        if destino.is_file() {
            fs::remove_file(&origen).contexto(|| format!("borrar {}", origen.display()))?;
            return Ok(());
        }
        fs::rename(&origen, &destino).contexto(|| format!("mover a {}", destino.display()))
    }

    /// Stores a copy of the `origen` file in the store and returns its hash.
    ///
    /// If `esperada` is given and the content does not match, nothing is stored and
    /// [`ErrorAlmacen::HuellaNoCoincide`] is returned. If the file was already stored, it is not duplicated.
    pub fn guardar_archivo(
        &self,
        origen: &Path,
        esperada: Option<&Huella>,
    ) -> Result<Huella, ErrorAlmacen> {
        let archivo = File::open(origen).contexto(|| format!("abrir {}", origen.display()))?;
        // We know its size: we check the space BEFORE starting to copy gigabytes.
        let tamano = archivo
            .metadata()
            .contexto(|| format!("consultar {}", origen.display()))?
            .len();
        self.comprobar_espacio(tamano)?;
        self.guardar_desde(archivo, esperada)
    }

    /// Stores everything read from `fuente` (a file, a download…) in the store.
    pub fn guardar_desde(
        &self,
        mut fuente: impl Read,
        esperada: Option<&Huella>,
    ) -> Result<Huella, ErrorAlmacen> {
        // 1. Copy to a temporary file while computing the hash (a single read).
        let temporal = Temporal::nuevo(&self.tmp)?;
        let huella = {
            let mut destino = File::create(temporal.ruta())
                .contexto(|| format!("crear {}", temporal.ruta().display()))?;
            let mut calculadora = Sha256::new();
            let mut buffer = vec![0u8; TROZO];
            loop {
                let leidos = fuente
                    .read(&mut buffer)
                    .contexto(|| "leer el archivo de origen".into())?;
                if leidos == 0 {
                    break;
                }
                calculadora.update(&buffer[..leidos]);
                destino
                    .write_all(&buffer[..leidos])
                    .contexto(|| "escribir el archivo temporal".into())?;
            }
            // Makes sure the data is on disk before accepting it.
            destino
                .sync_all()
                .contexto(|| "guardar en disco el archivo temporal".into())?;
            Huella::desde_bytes(&calculadora.finalize())
        }; // `destino` is closed here (Drop), before moving the file

        // 2. Verify.
        if let Some(esperada) = esperada
            && *esperada != huella
        {
            // `temporal` deletes itself on leaving (Drop).
            return Err(ErrorAlmacen::HuellaNoCoincide {
                esperada: esperada.clone(),
                obtenida: huella,
            });
        }

        // 3. Move to `blobs\` (or discard if it already existed: same content, same hash).
        let destino = self.ruta_blob(&huella);
        if !destino.is_file() {
            temporal.mover_a(&destino)?;
        }
        Ok(huella)
    }
}

/// Computes the sha256 hash of some bytes in memory.
pub fn huella_de_bytes(datos: &[u8]) -> Huella {
    Huella::desde_bytes(&Sha256::digest(datos))
}

/// Computes the sha256 hash of a file without copying it (for example, to verify an already downloaded model).
pub fn huella_de_archivo(ruta: &Path) -> Result<Huella, ErrorAlmacen> {
    let mut archivo = File::open(ruta).contexto(|| format!("abrir {}", ruta.display()))?;
    let mut calculadora = Sha256::new();
    let mut buffer = vec![0u8; TROZO];
    loop {
        let leidos = archivo
            .read(&mut buffer)
            .contexto(|| format!("leer {}", ruta.display()))?;
        if leidos == 0 {
            break;
        }
        calculadora.update(&buffer[..leidos]);
    }
    Ok(Huella::desde_bytes(&calculadora.finalize()))
}

// ─── Self-deleting temporary file ─────────────────────────────────────────────

/// A file in `tmp\` that is deleted automatically once no longer used, unless it is moved to its destination.
struct Temporal {
    ruta: Option<PathBuf>,
}

impl Temporal {
    fn nuevo(carpeta: &Path) -> Result<Temporal, ErrorAlmacen> {
        let numero = CONTADOR_TMP.fetch_add(1, Ordering::Relaxed);
        let nombre = format!("guardando-{}-{numero}.tmp", std::process::id());
        Ok(Temporal {
            ruta: Some(carpeta.join(nombre)),
        })
    }

    fn ruta(&self) -> &Path {
        self.ruta
            .as_deref()
            .expect("la ruta solo desaparece al mover el archivo")
    }

    /// Moves the file to its final destination. After this it is no longer deleted when dropped.
    fn mover_a(mut self, destino: &Path) -> Result<(), ErrorAlmacen> {
        let origen = self
            .ruta
            .take()
            .expect("la ruta solo desaparece al mover el archivo");
        fs::rename(&origen, destino).contexto(|| format!("mover a {}", destino.display()))?;
        Ok(())
    }
}

impl Drop for Temporal {
    fn drop(&mut self) {
        if let Some(ruta) = &self.ruta {
            let _ = fs::remove_file(ruta); // if it does not exist, nothing happens
        }
    }
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod pruebas {
    use super::*;

    /// sha256("abc"), the standard's official test vector (FIPS 180-2).
    const HUELLA_ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    /// A temporary folder for each test, deleted by itself at the end.
    struct CarpetaDePrueba(PathBuf);

    impl CarpetaDePrueba {
        fn nueva(nombre: &str) -> CarpetaDePrueba {
            let ruta =
                std::env::temp_dir().join(format!("locall-store-{nombre}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&ruta);
            fs::create_dir_all(&ruta).unwrap();
            CarpetaDePrueba(ruta)
        }
    }

    impl Drop for CarpetaDePrueba {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn archivos_en(carpeta: &Path) -> usize {
        fs::read_dir(carpeta).unwrap().count()
    }

    #[test]
    fn valida_las_huellas() {
        assert!(Huella::desde_hex(HUELLA_ABC).is_ok());
        assert!(Huella::desde_hex(&format!("sha256:{HUELLA_ABC}")).is_ok());
        let mayusculas = Huella::desde_hex(&HUELLA_ABC.to_uppercase()).unwrap();
        assert_eq!(mayusculas.hex(), HUELLA_ABC);

        for mala in [
            "",
            "abc",
            &HUELLA_ABC[..63],
            &format!("{HUELLA_ABC}0"),
            &"g".repeat(64),
        ] {
            assert!(
                Huella::desde_hex(mala).is_err(),
                "debería rechazar «{mala}»"
            );
        }
        // Attempts to escape the folder: impossible with this type.
        let escape = format!("..\\..\\{}", &HUELLA_ABC[6..]);
        assert!(Huella::desde_hex(&escape).is_err());
    }

    #[test]
    fn guarda_y_verifica_un_archivo() {
        let carpeta = CarpetaDePrueba::nueva("guarda");
        let almacen = Almacen::abrir(carpeta.0.join("almacen")).unwrap();
        let esperada = Huella::desde_hex(HUELLA_ABC).unwrap();

        let huella = almacen.guardar_desde(&b"abc"[..], Some(&esperada)).unwrap();

        assert_eq!(huella, esperada);
        assert!(almacen.contiene(&huella));
        assert_eq!(fs::read(almacen.ruta_blob(&huella)).unwrap(), b"abc");
        assert_eq!(archivos_en(&almacen.tmp), 0, "no deben quedar temporales");
    }

    #[test]
    fn descarta_un_archivo_que_no_coincide() {
        let carpeta = CarpetaDePrueba::nueva("descarta");
        let almacen = Almacen::abrir(carpeta.0.join("almacen")).unwrap();
        let esperada = Huella::desde_hex(HUELLA_ABC).unwrap();

        let resultado = almacen.guardar_desde(&b"abd"[..], Some(&esperada));

        assert!(matches!(
            resultado,
            Err(ErrorAlmacen::HuellaNoCoincide { .. })
        ));
        assert_eq!(archivos_en(&almacen.blobs), 0, "no debe guardarse nada");
        assert_eq!(archivos_en(&almacen.tmp), 0, "el temporal debe borrarse");
    }

    #[test]
    fn no_duplica_un_archivo_ya_guardado() {
        let carpeta = CarpetaDePrueba::nueva("duplica");
        let almacen = Almacen::abrir(carpeta.0.join("almacen")).unwrap();

        let primera = almacen.guardar_desde(&b"abc"[..], None).unwrap();
        let segunda = almacen.guardar_desde(&b"abc"[..], None).unwrap();

        assert_eq!(primera, segunda);
        assert_eq!(archivos_en(&almacen.blobs), 1);
        assert_eq!(archivos_en(&almacen.tmp), 0);
    }

    #[test]
    fn guarda_un_archivo_del_disco() {
        let carpeta = CarpetaDePrueba::nueva("disco");
        let origen = carpeta.0.join("origen.bin");
        let contenido: Vec<u8> = (0..3 * TROZO + 123).map(|i| (i % 251) as u8).collect(); // several chunks
        fs::write(&origen, &contenido).unwrap();
        let almacen = Almacen::abrir(carpeta.0.join("almacen")).unwrap();

        let huella = almacen.guardar_archivo(&origen, None).unwrap();

        assert_eq!(huella, huella_de_archivo(&origen).unwrap());
        assert_eq!(fs::read(almacen.ruta_blob(&huella)).unwrap(), contenido);
    }
}
