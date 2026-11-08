//! # locall-store
//!
//! El **almacén de modelos** de Locall.
//!
//! Cada archivo de modelo se guarda **una sola vez**, con su huella sha256 como nombre, y solo después de comprobar
//! que su contenido coincide con la huella esperada:
//!
//! ```text
//! <raíz>\
//! ├── blobs\sha256-<64 caracteres hexadecimales>   ← el contenido, verificado
//! ├── snapshots\...                                 ← (parte 3b) nombres originales, con enlaces duros
//! └── tmp\                                          ← archivos a medio guardar
//! ```
//!
//! Guardar es **atómico**: el archivo se escribe en `tmp\`, se verifica y solo entonces se mueve a `blobs\`.
//! Si algo falla a mitad, en `blobs\` nunca queda un archivo incompleto.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

/// Tamaño de los trozos con los que se lee y copia (1 MiB).
const TROZO: usize = 1024 * 1024;

// ─── Huella ───────────────────────────────────────────────────────────────────

/// Una huella sha256 **válida**: exactamente 64 caracteres hexadecimales en minúscula.
///
/// Es un *newtype*: un tipo propio que envuelve un `String`. Como el `String` interior es privado, la única forma
/// de obtener una `Huella` es con [`Huella::desde_hex`] (que valida) o calculándola. Así es **imposible** que una
/// huella contenga `..`, `\` u otros caracteres con los que construir una ruta fuera del almacén.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Huella(String);

impl Huella {
    /// Crea una huella a partir de texto. Admite el prefijo `sha256:` y mayúsculas (se pasan a minúscula).
    pub fn desde_hex(texto: &str) -> Result<Huella, ErrorAlmacen> {
        let hex = texto.strip_prefix("sha256:").unwrap_or(texto);
        if hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            Ok(Huella(hex.to_ascii_lowercase()))
        } else {
            Err(ErrorAlmacen::HuellaInvalida(texto.to_string()))
        }
    }

    /// Los 64 caracteres hexadecimales.
    pub fn hex(&self) -> &str {
        &self.0
    }

    /// El nombre del archivo en `blobs\`: `sha256-<hex>`.
    pub fn nombre_blob(&self) -> String {
        format!("sha256-{}", self.0)
    }

    /// Construye la huella a partir del resultado de sha256 (32 bytes).
    fn desde_bytes(bytes: &[u8]) -> Huella {
        Huella(bytes.iter().map(|b| format!("{b:02x}")).collect())
    }
}

impl fmt::Display for Huella {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sha256:{}", self.0)
    }
}

// ─── Errores ──────────────────────────────────────────────────────────────────

/// Lo que puede salir mal en el almacén.
#[derive(Debug)]
pub enum ErrorAlmacen {
    /// Error del sistema de archivos, con una descripción de lo que se intentaba hacer.
    Io { contexto: String, error: io::Error },
    /// El texto no es una huella sha256 válida.
    HuellaInvalida(String),
    /// El contenido no coincide con la huella esperada: el archivo está dañado o no es el que debía ser.
    HuellaNoCoincide { esperada: Huella, obtenida: Huella },
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

/// Añade contexto a los errores de E/S: `.contexto("abrir el archivo")`.
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

// ─── Almacén ──────────────────────────────────────────────────────────────────

/// El almacén de modelos, en una carpeta raíz.
#[derive(Debug, Clone)]
pub struct Almacen {
    raiz: PathBuf,
    blobs: PathBuf,
    tmp: PathBuf,
}

/// Contador para dar nombres únicos a los archivos temporales.
static CONTADOR_TMP: AtomicU64 = AtomicU64::new(0);

impl Almacen {
    /// Abre (y crea si no existe) un almacén en `raiz`.
    pub fn abrir(raiz: impl Into<PathBuf>) -> Result<Almacen, ErrorAlmacen> {
        let raiz = raiz.into();
        let blobs = raiz.join("blobs");
        let tmp = raiz.join("tmp");
        for carpeta in [&blobs, &tmp] {
            fs::create_dir_all(carpeta)
                .contexto(|| format!("crear la carpeta {}", carpeta.display()))?;
        }
        Ok(Almacen { raiz, blobs, tmp })
    }

    /// La carpeta raíz del almacén.
    pub fn raiz(&self) -> &Path {
        &self.raiz
    }

    /// Dónde está (o estaría) el archivo con esta huella.
    pub fn ruta_blob(&self, huella: &Huella) -> PathBuf {
        self.blobs.join(huella.nombre_blob())
    }

    /// ¿Ya tenemos un archivo con esta huella?
    pub fn contiene(&self, huella: &Huella) -> bool {
        self.ruta_blob(huella).is_file()
    }

    /// Guarda una copia del archivo `origen` en el almacén y devuelve su huella.
    ///
    /// Si se indica `esperada` y el contenido no coincide, no se guarda nada y se devuelve
    /// [`ErrorAlmacen::HuellaNoCoincide`]. Si el archivo ya estaba guardado, no se duplica.
    pub fn guardar_archivo(
        &self,
        origen: &Path,
        esperada: Option<&Huella>,
    ) -> Result<Huella, ErrorAlmacen> {
        let archivo = File::open(origen).contexto(|| format!("abrir {}", origen.display()))?;
        self.guardar_desde(archivo, esperada)
    }

    /// Guarda en el almacén todo lo que se lea de `fuente` (un archivo, una descarga…).
    pub fn guardar_desde(
        &self,
        mut fuente: impl Read,
        esperada: Option<&Huella>,
    ) -> Result<Huella, ErrorAlmacen> {
        // 1. Copiar a un archivo temporal mientras se calcula la huella (una sola lectura).
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
            // Asegura que los datos están en el disco antes de darlos por buenos.
            destino
                .sync_all()
                .contexto(|| "guardar en disco el archivo temporal".into())?;
            Huella::desde_bytes(&calculadora.finalize())
        }; // aquí se cierra `destino` (Drop), antes de mover el archivo

        // 2. Verificar.
        if let Some(esperada) = esperada
            && *esperada != huella
        {
            // `temporal` se borra solo al salir (Drop).
            return Err(ErrorAlmacen::HuellaNoCoincide {
                esperada: esperada.clone(),
                obtenida: huella,
            });
        }

        // 3. Mover a `blobs\` (o descartar si ya existía: mismo contenido, misma huella).
        let destino = self.ruta_blob(&huella);
        if !destino.is_file() {
            temporal.mover_a(&destino)?;
        }
        Ok(huella)
    }
}

/// Calcula la huella sha256 de un archivo sin copiarlo (por ejemplo, para verificar un modelo ya descargado).
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

// ─── Archivo temporal que se borra solo ───────────────────────────────────────

/// Un archivo en `tmp\` que se borra automáticamente al dejar de usarse, salvo que se mueva a su destino.
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

    /// Mueve el archivo a su destino definitivo. Después de esto ya no se borra al soltarlo.
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
            let _ = fs::remove_file(ruta); // si no existe, no pasa nada
        }
    }
}

// ─── Pruebas unitarias ────────────────────────────────────────────────────────

#[cfg(test)]
mod pruebas {
    use super::*;

    /// sha256("abc"), el vector de prueba oficial del estándar (FIPS 180-2).
    const HUELLA_ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    /// Una carpeta temporal para cada prueba, que se borra sola al terminar.
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
        // Intentos de escaparse de la carpeta: imposibles con este tipo.
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
        let contenido: Vec<u8> = (0..3 * TROZO + 123).map(|i| (i % 251) as u8).collect(); // varios trozos
        fs::write(&origen, &contenido).unwrap();
        let almacen = Almacen::abrir(carpeta.0.join("almacen")).unwrap();

        let huella = almacen.guardar_archivo(&origen, None).unwrap();

        assert_eq!(huella, huella_de_archivo(&origen).unwrap());
        assert_eq!(fs::read(almacen.ruta_blob(&huella)).unwrap(), contenido);
    }
}
