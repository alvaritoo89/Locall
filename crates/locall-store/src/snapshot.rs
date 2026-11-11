//! Snapshot folders: each file of a model with its **original name**, as a hard link to its blob.
//!
//! ```text
//! snapshots\qwen2.5-7b\bb5d59e0\
//! ├── qwen2.5-7b-instruct-q4_k_m-00001-of-00002.gguf   → blobs\sha256-dfce12…
//! └── qwen2.5-7b-instruct-q4_k_m-00002-of-00002.gguf   → blobs\sha256-539cf9…
//! ```
//!
//! A **hard link** is another name for the same file: it takes no extra space and the content is not deleted until
//! the last name is removed (checked in Phase 0). Hard links cannot cross from one disk to another, which is why
//! blobs and snapshots always live in the same root folder.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use crate::{Almacen, CONTADOR_TMP, ConContexto, ErrorAlmacen, Huella, NombreArchivo, Segmento};

/// A snapshot folder already created.
#[derive(Debug, Clone)]
pub struct Snapshot {
    carpeta: PathBuf,
    archivos: Vec<PathBuf>,
}

impl Snapshot {
    /// The snapshot folder.
    pub fn carpeta(&self) -> &Path {
        &self.carpeta
    }

    /// All the files, sorted by name.
    pub fn archivos(&self) -> &[PathBuf] {
        &self.archivos
    }

    /// The file to pass to llama.cpp: the first one by name (in a split model, the `-00001-of-…` part; llama.cpp finds
    /// the others by their name).
    pub fn ruta_principal(&self) -> &Path {
        &self.archivos[0]
    }
}

impl Almacen {
    /// Where the snapshot folder of a model and revision is (or would be).
    pub fn carpeta_snapshot(&self, modelo: &Segmento, revision: &Segmento) -> PathBuf {
        self.snapshots
            .join(modelo.como_str())
            .join(revision.como_str())
    }

    /// Creates (or recreates) a model's snapshot folder with its files.
    ///
    /// It is **atomic**: it is built in a temporary folder and only put in place at the end. If a blob does not exist or
    /// something fails, no half-built snapshot is left.
    pub fn crear_snapshot(
        &self,
        modelo: &Segmento,
        revision: &Segmento,
        archivos: &[(NombreArchivo, Huella)],
    ) -> Result<Snapshot, ErrorAlmacen> {
        if archivos.is_empty() {
            return Err(ErrorAlmacen::SnapshotVacio);
        }
        // Check BEFORE touching anything that all the blobs exist.
        for (_, huella) in archivos {
            if !self.contiene(huella) {
                return Err(ErrorAlmacen::BlobNoEncontrado(huella.clone()));
            }
        }

        let carpeta_modelo = self.snapshots.join(modelo.como_str());
        fs::create_dir_all(&carpeta_modelo)
            .contexto(|| format!("crear la carpeta {}", carpeta_modelo.display()))?;

        // 1. Build in a temporary folder inside the model's (same disk, so it can be renamed).
        let numero = CONTADOR_TMP.fetch_add(1, Ordering::Relaxed);
        let temporal = carpeta_modelo.join(format!(
            ".creando-{}-{}-{numero}",
            revision,
            std::process::id()
        ));
        let resultado = self.enlazar_en(&temporal, archivos);
        if let Err(e) = resultado {
            let _ = fs::remove_dir_all(&temporal);
            return Err(e);
        }

        // 2. Replace the previous snapshot, if there was one, and put the new one in place.
        let definitiva = carpeta_modelo.join(revision.como_str());
        if definitiva.exists() {
            fs::remove_dir_all(&definitiva)
                .contexto(|| format!("borrar la snapshot anterior {}", definitiva.display()))?;
        }
        fs::rename(&temporal, &definitiva)
            .contexto(|| format!("colocar la snapshot en {}", definitiva.display()))?;

        let mut rutas: Vec<PathBuf> = archivos
            .iter()
            .map(|(nombre, _)| definitiva.join(nombre.como_str()))
            .collect();
        rutas.sort();
        Ok(Snapshot {
            carpeta: definitiva,
            archivos: rutas,
        })
    }

    /// Creates one hard link per file in `carpeta`.
    fn enlazar_en(
        &self,
        carpeta: &Path,
        archivos: &[(NombreArchivo, Huella)],
    ) -> Result<(), ErrorAlmacen> {
        fs::create_dir_all(carpeta)
            .contexto(|| format!("crear la carpeta {}", carpeta.display()))?;
        for (nombre, huella) in archivos {
            let destino = carpeta.join(nombre.como_str());
            fs::hard_link(self.ruta_blob(huella), &destino)
                .contexto(|| format!("crear el enlace {}", destino.display()))?;
        }
        Ok(())
    }

    /// Deletes a model's snapshot folder. The blobs are kept (other models could use them);
    /// cleanup of unused blobs will come in part 3c.
    pub fn eliminar_snapshot(
        &self,
        modelo: &Segmento,
        revision: &Segmento,
    ) -> Result<(), ErrorAlmacen> {
        let carpeta = self.carpeta_snapshot(modelo, revision);
        if carpeta.exists() {
            fs::remove_dir_all(&carpeta)
                .contexto(|| format!("borrar la snapshot {}", carpeta.display()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod pruebas {
    use std::io::Write;

    use super::*;

    struct CarpetaDePrueba(PathBuf);

    impl CarpetaDePrueba {
        fn nueva(nombre: &str) -> CarpetaDePrueba {
            let ruta = std::env::temp_dir()
                .join(format!("locall-snapshot-{nombre}-{}", std::process::id()));
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

    fn seg(texto: &str) -> Segmento {
        Segmento::nuevo(texto).unwrap()
    }

    fn nombre(texto: &str) -> NombreArchivo {
        NombreArchivo::nuevo(texto).unwrap()
    }

    #[test]
    fn crea_una_snapshot_con_enlaces_duros() {
        let carpeta = CarpetaDePrueba::nueva("crea");
        let almacen = Almacen::abrir(carpeta.0.join("almacen")).unwrap();
        let huella = almacen.guardar_desde(&b"abc"[..], None).unwrap();

        let snap = almacen
            .crear_snapshot(
                &seg("prueba"),
                &seg("rev1"),
                &[(nombre("modelo.gguf"), huella.clone())],
            )
            .unwrap();

        assert_eq!(fs::read(snap.ruta_principal()).unwrap(), b"abc");

        // Proof that it is THE SAME file (hard link) and not a copy: what is written through one name shows up through
        // the other. (Locall never writes to models; this is only for the test.)
        let mut f = fs::OpenOptions::new()
            .append(true)
            .open(snap.ruta_principal())
            .unwrap();
        f.write_all(b"X").unwrap();
        drop(f);
        assert_eq!(fs::read(almacen.ruta_blob(&huella)).unwrap(), b"abcX");
    }

    #[test]
    fn ordena_las_partes_de_un_modelo_dividido() {
        let carpeta = CarpetaDePrueba::nueva("partes");
        let almacen = Almacen::abrir(carpeta.0.join("almacen")).unwrap();
        let h1 = almacen.guardar_desde(&b"parte1"[..], None).unwrap();
        let h2 = almacen.guardar_desde(&b"parte2"[..], None).unwrap();

        // Deliberately passed out of order.
        let snap = almacen
            .crear_snapshot(
                &seg("m"),
                &seg("r"),
                &[
                    (nombre("m-00002-of-00002.gguf"), h2),
                    (nombre("m-00001-of-00002.gguf"), h1),
                ],
            )
            .unwrap();

        assert_eq!(snap.archivos().len(), 2);
        assert!(snap.ruta_principal().ends_with("m-00001-of-00002.gguf"));
    }

    #[test]
    fn no_deja_nada_si_falta_un_blob() {
        let carpeta = CarpetaDePrueba::nueva("falta");
        let almacen = Almacen::abrir(carpeta.0.join("almacen")).unwrap();
        let existente = almacen.guardar_desde(&b"abc"[..], None).unwrap();
        let inexistente = Huella::desde_hex(&"0".repeat(64)).unwrap();

        let resultado = almacen.crear_snapshot(
            &seg("m"),
            &seg("r"),
            &[
                (nombre("a.gguf"), existente),
                (nombre("b.gguf"), inexistente),
            ],
        );

        assert!(matches!(resultado, Err(ErrorAlmacen::BlobNoEncontrado(_))));
        assert!(!almacen.carpeta_snapshot(&seg("m"), &seg("r")).exists());
    }

    #[test]
    fn se_puede_rehacer_y_eliminar_sin_perder_el_blob() {
        let carpeta = CarpetaDePrueba::nueva("rehacer");
        let almacen = Almacen::abrir(carpeta.0.join("almacen")).unwrap();
        let huella = almacen.guardar_desde(&b"abc"[..], None).unwrap();
        let archivos = [(nombre("modelo.gguf"), huella.clone())];

        almacen
            .crear_snapshot(&seg("m"), &seg("r"), &archivos)
            .unwrap();
        almacen
            .crear_snapshot(&seg("m"), &seg("r"), &archivos)
            .unwrap(); // again: it does not fail

        almacen.eliminar_snapshot(&seg("m"), &seg("r")).unwrap();
        assert!(!almacen.carpeta_snapshot(&seg("m"), &seg("r")).exists());
        assert!(almacen.contiene(&huella), "el blob debe seguir ahí");
        // No temporary build folders are left.
        let restos = fs::read_dir(almacen.raiz().join("snapshots").join("m"))
            .unwrap()
            .count();
        assert_eq!(restos, 0);
    }
}
