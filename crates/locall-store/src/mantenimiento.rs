//! Mantenimiento del almacén: espacio libre, aviso de OneDrive y limpieza.
//!
//! Regla 14 del proyecto: nada se borra sin autorización. Por eso la limpieza se hace en dos pasos: primero se
//! **lista** lo que se podría borrar ([`Almacen::blobs_sin_usar`]) y después se borra **uno a uno** lo que el
//! usuario apruebe ([`Almacen::eliminar_blob`]), comprobándolo de nuevo justo antes.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::{Almacen, ConContexto, ErrorAlmacen, Huella, sistema};

/// Margen de espacio libre que siempre se deja en el disco (1 GiB), para no llenarlo del todo.
pub const MARGEN_ESPACIO: u64 = 1024 * 1024 * 1024;

/// Antigüedad a partir de la cual un temporal se considera abandonado (por ejemplo, tras un corte de luz).
const TEMPORAL_ABANDONADO: Duration = Duration::from_secs(60 * 60);

/// Un archivo del almacén que ningún modelo usa y que se podría borrar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobSinUsar {
    pub huella: Huella,
    pub bytes: u64,
}

impl Almacen {
    /// Bytes libres en el disco del almacén.
    pub fn espacio_libre(&self) -> Result<u64, ErrorAlmacen> {
        sistema::espacio_libre(&self.raiz)
            .contexto(|| format!("consultar el espacio libre de {}", self.raiz.display()))
    }

    /// Comprueba que caben `necesarios` bytes dejando el margen de seguridad ([`MARGEN_ESPACIO`]).
    pub fn comprobar_espacio(&self, necesarios: u64) -> Result<(), ErrorAlmacen> {
        let disponibles = self.espacio_libre()?;
        if disponibles < necesarios.saturating_add(MARGEN_ESPACIO) {
            return Err(ErrorAlmacen::EspacioInsuficiente {
                necesarios,
                disponibles,
            });
        }
        Ok(())
    }

    /// ¿Está la carpeta del almacén dentro de OneDrive? Si es así, la app debe avisar: OneDrive intentaría subir
    /// gigas de modelos a la nube.
    pub fn esta_en_onedrive(&self) -> bool {
        let carpetas: Vec<PathBuf> = ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"]
            .iter()
            .filter_map(std::env::var_os)
            .filter(|v: &OsString| !v.is_empty())
            .map(PathBuf::from)
            .collect();
        ruta_dentro_de(&self.raiz, &carpetas)
    }

    /// Lista los blobs que no usa ningún modelo: los que solo tienen un nombre (el suyo en `blobs\`).
    ///
    /// Un blob adoptado de un archivo externo tiene dos nombres (el original y el del almacén) y no aparece aquí:
    /// borrarlo no liberaría espacio.
    pub fn blobs_sin_usar(&self) -> Result<Vec<BlobSinUsar>, ErrorAlmacen> {
        let mut resultado = Vec::new();
        let entradas = fs::read_dir(&self.blobs)
            .contexto(|| format!("leer la carpeta {}", self.blobs.display()))?;
        for entrada in entradas {
            let entrada = entrada.contexto(|| "leer una entrada de blobs".into())?;
            let nombre = entrada.file_name();
            // Solo nos interesan los archivos con nombre de blob válido; cualquier otra cosa se ignora.
            let Some(huella) = nombre
                .to_str()
                .and_then(|n| n.strip_prefix("sha256-"))
                .and_then(|hex| Huella::desde_hex(hex).ok())
            else {
                continue;
            };
            let ruta = entrada.path();
            let enlaces = sistema::numero_de_enlaces(&ruta)
                .contexto(|| format!("consultar {}", ruta.display()))?;
            if enlaces == 1 {
                let bytes = entrada
                    .metadata()
                    .contexto(|| format!("consultar {}", ruta.display()))?
                    .len();
                resultado.push(BlobSinUsar { huella, bytes });
            }
        }
        resultado.sort_by(|a, b| a.huella.cmp(&b.huella));
        Ok(resultado)
    }

    /// Borra un blob **solo si sigue sin usarse** (se comprueba otra vez justo antes de borrar).
    pub fn eliminar_blob(&self, huella: &Huella) -> Result<u64, ErrorAlmacen> {
        let ruta = self.ruta_blob(huella);
        if !ruta.is_file() {
            return Err(ErrorAlmacen::BlobNoEncontrado(huella.clone()));
        }
        let enlaces = sistema::numero_de_enlaces(&ruta)
            .contexto(|| format!("consultar {}", ruta.display()))?;
        if enlaces != 1 {
            return Err(ErrorAlmacen::BlobEnUso(huella.clone()));
        }
        let bytes = fs::metadata(&ruta)
            .contexto(|| format!("consultar {}", ruta.display()))?
            .len();
        fs::remove_file(&ruta).contexto(|| format!("borrar {}", ruta.display()))?;
        Ok(bytes)
    }

    /// Borra los temporales abandonados (de hace más de una hora): restos de un corte de luz o de un cierre brusco.
    /// Los recientes se dejan, por si otra operación los está usando. Devuelve cuántos se han borrado.
    pub fn limpiar_temporales(&self) -> Result<usize, ErrorAlmacen> {
        let ahora = SystemTime::now();
        let mut borrados = 0;
        let entradas = fs::read_dir(&self.tmp)
            .contexto(|| format!("leer la carpeta {}", self.tmp.display()))?;
        for entrada in entradas {
            let entrada = entrada.contexto(|| "leer una entrada de tmp".into())?;
            let ruta = entrada.path();
            // Las descargas pausadas (`.parcial`) NO son basura: se reanudan la próxima vez. Solo se borran si el
            // usuario cancela la descarga.
            if ruta.extension().is_some_and(|e| e == "parcial") {
                continue;
            }
            let Ok(metadatos) = entrada.metadata() else {
                continue;
            };
            let antiguo = metadatos
                .modified()
                .ok()
                .and_then(|m| ahora.duration_since(m).ok())
                .is_some_and(|edad| edad > TEMPORAL_ABANDONADO);
            if metadatos.is_file() && antiguo && fs::remove_file(&ruta).is_ok() {
                borrados += 1;
            }
        }
        Ok(borrados)
    }
}

/// ¿Está `ruta` dentro de alguna de `carpetas`? Compara sin distinguir mayúsculas (como Windows) y por
/// componentes completos (`C:\OneDriveX` no está dentro de `C:\OneDrive`).
pub(crate) fn ruta_dentro_de(ruta: &Path, carpetas: &[PathBuf]) -> bool {
    let normalizar = |p: &Path| -> Vec<String> {
        let absoluta = fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        absoluta
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
            .collect()
    };
    let ruta = normalizar(ruta);
    carpetas.iter().any(|carpeta| {
        let carpeta = normalizar(carpeta);
        !carpeta.is_empty() && ruta.starts_with(&carpeta)
    })
}

#[cfg(test)]
mod pruebas {
    use std::fs::File;

    use super::*;
    use crate::{NombreArchivo, Segmento};

    struct CarpetaDePrueba(PathBuf);

    impl CarpetaDePrueba {
        fn nueva(nombre: &str) -> CarpetaDePrueba {
            let ruta =
                std::env::temp_dir().join(format!("locall-mant-{nombre}-{}", std::process::id()));
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

    #[test]
    fn consulta_el_espacio_libre() {
        let carpeta = CarpetaDePrueba::nueva("espacio");
        let almacen = Almacen::abrir(carpeta.0.join("a")).unwrap();
        assert!(almacen.espacio_libre().unwrap() > 0);
        assert!(almacen.comprobar_espacio(1).is_ok());
        // Nadie tiene 16 exabytes libres.
        assert!(matches!(
            almacen.comprobar_espacio(u64::MAX / 2),
            Err(ErrorAlmacen::EspacioInsuficiente { .. })
        ));
    }

    #[test]
    fn detecta_rutas_dentro_de_onedrive() {
        let onedrive = vec![PathBuf::from(r"C:\Users\ana\OneDrive")];
        assert!(ruta_dentro_de(
            Path::new(r"C:\Users\ana\OneDrive\Modelos"),
            &onedrive
        ));
        assert!(ruta_dentro_de(
            Path::new(r"c:\users\ANA\onedrive\modelos"),
            &onedrive
        ));
        assert!(!ruta_dentro_de(
            Path::new(r"C:\Users\ana\OneDriveX\Modelos"),
            &onedrive
        ));
        assert!(!ruta_dentro_de(Path::new(r"C:\Locall\modelos"), &onedrive));
        assert!(!ruta_dentro_de(Path::new(r"C:\Locall\modelos"), &[]));
    }

    #[test]
    fn lista_y_borra_solo_los_blobs_sin_usar() {
        let carpeta = CarpetaDePrueba::nueva("limpieza");
        let almacen = Almacen::abrir(carpeta.0.join("a")).unwrap();
        let usado = almacen.guardar_desde(&b"usado"[..], None).unwrap();
        let libre = almacen.guardar_desde(&b"libre!"[..], None).unwrap();
        almacen
            .crear_snapshot(
                &Segmento::nuevo("m").unwrap(),
                &Segmento::nuevo("r").unwrap(),
                &[(NombreArchivo::nuevo("m.gguf").unwrap(), usado.clone())],
            )
            .unwrap();

        let sin_usar = almacen.blobs_sin_usar().unwrap();
        assert_eq!(
            sin_usar,
            vec![BlobSinUsar {
                huella: libre.clone(),
                bytes: 6
            }]
        );

        // El que usa un modelo NO se puede borrar.
        assert!(matches!(
            almacen.eliminar_blob(&usado),
            Err(ErrorAlmacen::BlobEnUso(_))
        ));
        // El que no usa nadie, sí.
        assert_eq!(almacen.eliminar_blob(&libre).unwrap(), 6);
        assert!(!almacen.contiene(&libre));
        assert!(almacen.contiene(&usado));
    }

    #[test]
    fn borra_solo_los_temporales_abandonados() {
        let carpeta = CarpetaDePrueba::nueva("tmp");
        let almacen = Almacen::abrir(carpeta.0.join("a")).unwrap();
        let tmp = almacen.raiz().join("tmp");
        let viejo = tmp.join("viejo.tmp");
        let reciente = tmp.join("reciente.tmp");
        let pausada = tmp.join("sha256-abc.parcial"); // una descarga pausada, también antigua
        for ruta in [&viejo, &reciente, &pausada] {
            fs::write(ruta, b"x").unwrap();
        }
        let hace_dos_horas = SystemTime::now() - Duration::from_secs(2 * 60 * 60);
        for ruta in [&viejo, &pausada] {
            File::options()
                .write(true)
                .open(ruta)
                .unwrap()
                .set_modified(hace_dos_horas)
                .unwrap();
        }

        assert_eq!(almacen.limpiar_temporales().unwrap(), 1);
        assert!(!viejo.exists());
        assert!(reciente.exists());
        assert!(pausada.exists(), "una descarga pausada no se borra");
    }

    #[test]
    fn confirma_una_descarga_verificada() {
        let carpeta = CarpetaDePrueba::nueva("confirmar");
        let almacen = Almacen::abrir(carpeta.0.join("a")).unwrap();
        let huella = crate::huella_de_bytes(b"abc");
        fs::write(almacen.ruta_parcial(&huella), b"abc").unwrap();

        almacen.confirmar_parcial(&huella).unwrap();

        assert!(almacen.contiene(&huella));
        assert!(!almacen.ruta_parcial(&huella).exists());
    }
}
