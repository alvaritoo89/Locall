//! The catalog of compatible models inside the core (ADR 0022 and 0023).
//!
//! - **On opening**, Locall uses the newest catalog it has with a good signature: the one shipped inside it
//!   ([`locall_catalogo::incluido`]) or one downloaded earlier (in `<data>\catalogo\`).
//! - [`Nucleo::actualizar_catalogo`] checks GitHub for a new one, **at most once a day** (unless forced). It only
//!   accepts it if the signature is good **and** it is newer than the one it already has (`published`): this way
//!   nobody can push an old catalog back onto Locall even if it is properly signed.
//! - [`Nucleo::instalar_del_catalogo`] installs by name (`qwen3-4b`) and, **before downloading**, checks that what
//!   Hugging Face offers at that commit matches exactly (paths, sizes and sha256) what is signed in the catalog.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use locall_catalogo::{Catalogo, ErrorCatalogo, ModeloCatalogo, TAMANO_MAX, Variante};
use locall_registry::Modelo;

use crate::{ErrorNucleo, Nucleo, PeticionInstalar, ProgresoInstalacion, elegir_partes};

/// Where the official catalog is (the file and, next to it, its `.minisig` signature).
pub const URL_CATALOGO: &str =
    "https://raw.githubusercontent.com/alvaritoo89/Locall/main/catalogo/catalogo.json";

/// At most how often to check for a new catalog.
pub const INTERVALO_COMPROBACION: Duration = Duration::from_secs(24 * 60 * 60);

/// A minisign signature takes about 300 bytes: anything larger is not a signature.
const FIRMA_MAX: u64 = 4096;

/// Where the catalog in use came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrigenCatalogo {
    /// The one shipped inside this Locall.
    Incluido,
    /// A newer one, downloaded from GitHub.
    Descargado,
}

/// The catalog in use and when it was last checked whether there was another one.
#[derive(Debug, Clone)]
pub struct EstadoCatalogo {
    pub catalogo: Catalogo,
    pub origen: OrigenCatalogo,
    /// Seconds since 1970 of the last time GitHub was checked (`None`: never).
    pub comprobado: Option<u64>,
}

/// What happened when looking for a new catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actualizacion {
    /// It was checked less than a day ago: it has not been checked again.
    NoTocaba,
    /// It was checked and the current one is the same or newer.
    SinNovedades,
    /// There was a new one with a good signature: it is already in use.
    Actualizado,
}

const ARCHIVO: &str = "catalogo.json";
const FIRMA: &str = "catalogo.json.minisig";
const COMPROBADO: &str = "comprobado";

/// The catalog the core starts with: the newest between the shipped one and the saved one (if its signature is
/// good).
pub(crate) fn cargar(carpeta: &Path, clave: &str) -> EstadoCatalogo {
    let incluido = locall_catalogo::incluido().ok();
    let guardado = std::fs::read(carpeta.join(ARCHIVO))
        .ok()
        .zip(std::fs::read_to_string(carpeta.join(FIRMA)).ok())
        .and_then(
            |(datos, firma)| match locall_catalogo::verificar(&datos, &firma, clave) {
                Ok(c) => Some(c),
                Err(e) => {
                    log::warn!("el catálogo guardado no sirve ({e}); se usa el que lleva Locall");
                    None
                }
            },
        );
    let comprobado = std::fs::read_to_string(carpeta.join(COMPROBADO))
        .ok()
        .and_then(|t| t.trim().parse().ok());
    let (catalogo, origen) = match (incluido, guardado) {
        (Some(i), Some(g)) if g.publicado > i.publicado => (g, OrigenCatalogo::Descargado),
        (Some(i), _) => (i, OrigenCatalogo::Incluido),
        (None, Some(g)) => (g, OrigenCatalogo::Descargado),
        (None, None) => {
            // Would only happen if Locall were built with a badly signed catalog (a test prevents it).
            log::warn!("no hay ningún catálogo con firma buena");
            (
                Catalogo {
                    publicado: 0,
                    modelos: Vec::new(),
                },
                OrigenCatalogo::Incluido,
            )
        }
    };
    EstadoCatalogo {
        catalogo,
        origen,
        comprobado,
    }
}

impl Nucleo {
    /// The catalog in use.
    pub fn catalogo(&self) -> EstadoCatalogo {
        self.estado_catalogo().clone()
    }

    /// Checks GitHub for a new catalog (at most once a day, unless `forzar`).
    pub fn actualizar_catalogo(&self, forzar: bool) -> Result<Actualizacion, ErrorNucleo> {
        let ahora = locall_registry::ahora();
        {
            let estado = self.estado_catalogo();
            if !forzar
                && estado
                    .comprobado
                    .is_some_and(|c| ahora.saturating_sub(c) < INTERVALO_COMPROBACION.as_secs())
            {
                return Ok(Actualizacion::NoTocaba);
            }
        }
        let carpeta = self.rutas.catalogo();
        // The check is recorded even if it fails, so as not to insist every time if GitHub does not answer.
        self.estado_catalogo().comprobado = Some(ahora);
        let _ = std::fs::create_dir_all(&carpeta);
        let _ = std::fs::write(carpeta.join(COMPROBADO), ahora.to_string());

        let datos = self.descargar_limitado(&self.url_catalogo, TAMANO_MAX as u64)?;
        let firma =
            self.descargar_limitado(&format!("{}.minisig", self.url_catalogo), FIRMA_MAX)?;
        let firma = String::from_utf8(firma)
            .map_err(|_| ErrorNucleo::Catalogo(ErrorCatalogo::FirmaIlegible))?;
        let nuevo =
            locall_catalogo::verificar(&datos, &firma, &self.clave_catalogo).map_err(|e| {
                log::warn!("catálogo descargado rechazado: {e}");
                ErrorNucleo::Catalogo(e)
            })?;

        let mut estado = self.estado_catalogo();
        if nuevo.publicado <= estado.catalogo.publicado {
            if nuevo.publicado < estado.catalogo.publicado {
                log::warn!(
                    "el catálogo descargado es más antiguo que el que hay ({} < {}); se ignora",
                    nuevo.publicado,
                    estado.catalogo.publicado
                );
            }
            return Ok(Actualizacion::SinNovedades);
        }
        // Save first (temporary file + rename): if it is cut halfway, on opening the signature does not match and the
        // shipped one is used; never a half-written one.
        escribir_seguro(&carpeta.join(ARCHIVO), &datos)?;
        escribir_seguro(&carpeta.join(FIRMA), firma.as_bytes())?;
        log::info!(
            "catálogo actualizado: {} modelos (publicado {})",
            nuevo.modelos.len(),
            nuevo.publicado
        );
        estado.catalogo = nuevo;
        estado.origen = OrigenCatalogo::Descargado;
        Ok(Actualizacion::Actualizado)
    }

    /// Installs a catalog model by its name (`qwen3-4b` or `qwen3-4b:q8_0`).
    ///
    /// The identifier in Locall is the model name for the default variant (`qwen3-4b`) and `name-variant` for the others
    /// (`qwen3-4b-q8_0`).
    pub fn instalar_del_catalogo(
        &self,
        texto: &str,
        cancelar: &AtomicBool,
        progreso: impl FnMut(ProgresoInstalacion),
    ) -> Result<Modelo, ErrorNucleo> {
        let (modelo, variante) = {
            let estado = self.estado_catalogo();
            let (m, v) = estado
                .catalogo
                .resolver(texto)
                .ok_or_else(|| ErrorNucleo::NoEnCatalogo(texto.trim().to_owned()))?;
            (m.clone(), v.clone())
        };
        self.comprobar_contra_hugging_face(&modelo, &variante)?;
        let id = if variante.por_defecto {
            modelo.nombre.clone()
        } else {
            format!("{}-{}", modelo.nombre, variante.nombre)
        };
        self.instalar(
            &PeticionInstalar {
                repo: variante.repo.clone(),
                revision: variante.commit.clone(),
                archivo: variante.archivos[0].ruta.clone(),
                id: Some(id),
            },
            cancelar,
            progreso,
        )
    }

    /// What Hugging Face has at that commit must be **exactly** what is signed in the catalog: the same parts, with the
    /// same size and the same sha256. Otherwise, nothing is downloaded.
    fn comprobar_contra_hugging_face(
        &self,
        modelo: &ModeloCatalogo,
        variante: &Variante,
    ) -> Result<(), ErrorNucleo> {
        let (info, ggufs) = self.consultar(&variante.repo, &variante.commit)?;
        let no_coincide =
            |que: String| ErrorNucleo::CatalogoNoCoincide(format!("{}: {que}", modelo.nombre));
        if info.commit != variante.commit {
            return Err(no_coincide("Hugging Face devuelve otro commit".into()));
        }
        let partes = elegir_partes(&ggufs, &variante.archivos[0].ruta)?;
        if partes.len() != variante.archivos.len() {
            return Err(no_coincide(format!(
                "el catálogo dice {} partes y Hugging Face tiene {}",
                variante.archivos.len(),
                partes.len()
            )));
        }
        for esperado in &variante.archivos {
            let real = partes
                .iter()
                .find(|p| p.ruta == esperado.ruta)
                .ok_or_else(|| no_coincide(format!("falta «{}»", esperado.ruta)))?;
            if real.bytes != esperado.bytes {
                return Err(no_coincide(format!(
                    "«{}» no tiene el tamaño firmado",
                    esperado.ruta
                )));
            }
            if real.huella.as_ref().map(|h| h.hex()) != Some(esperado.sha256.as_str()) {
                return Err(no_coincide(format!(
                    "«{}» no tiene la huella firmada",
                    esperado.ruta
                )));
            }
        }
        Ok(())
    }

    /// Would a model of `bytes` fit on this computer? **Approximate**: without the file its header cannot be read, so it
    /// is estimated as the file + 10 % + 512 MB (context and engine). Returns the estimated memory and the fit.
    /// As in [`Nucleo::evaluar`], the memory of the loaded model counts as available (it would be replaced).
    pub fn encaje_aproximado(&self, bytes: u64) -> (u64, locall_compat::Encaje) {
        let estimado = bytes
            .saturating_add(bytes / 10)
            .saturating_add(512 * 1024 * 1024);
        let disponible = locall_system::memoria()
            .map(|m| m.disponible)
            .unwrap_or(0)
            .saturating_add(self.memoria_del_modelo().unwrap_or(0));
        (estimado, locall_compat::encaje(estimado, disponible))
    }

    fn estado_catalogo(&self) -> std::sync::MutexGuard<'_, EstadoCatalogo> {
        self.catalogo
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Downloads a small file, without exceeding `maximo` bytes.
    fn descargar_limitado(&self, url: &str, maximo: u64) -> Result<Vec<u8>, ErrorNucleo> {
        let respuesta = self
            .agente_catalogo
            .get(url)
            .call()
            .map_err(|e| ErrorNucleo::Red(e.to_string()))?;
        let estado = respuesta.status().as_u16();
        if estado != 200 {
            return Err(ErrorNucleo::CatalogoNoDisponible(estado));
        }
        respuesta
            .into_body()
            .with_config()
            .limit(maximo)
            .read_to_vec()
            .map_err(|e| ErrorNucleo::Red(e.to_string()))
    }
}

/// Writes to a temporary file and renames it over the destination.
fn escribir_seguro(ruta: &Path, datos: &[u8]) -> Result<(), ErrorNucleo> {
    let temporal = ruta.with_extension("nuevo");
    let error =
        |e: std::io::Error| ErrorNucleo::Red(format!("no se pudo guardar el catálogo: {e}"));
    std::fs::write(&temporal, datos).map_err(error)?;
    std::fs::rename(&temporal, ruta).map_err(|e| {
        let _ = std::fs::remove_file(&temporal);
        error(e)
    })
}
