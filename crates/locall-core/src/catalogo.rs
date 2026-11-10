//! El catálogo de modelos compatibles dentro del núcleo (ADR 0022 y 0023).
//!
//! - **Al abrir**, Locall usa el catálogo más nuevo que tenga con firma buena: el que lleva dentro
//!   ([`locall_catalogo::incluido`]) o uno descargado antes (en `<datos>\catalogo\`).
//! - [`Nucleo::actualizar_catalogo`] mira en GitHub si hay uno nuevo, **como mucho una vez al día** (salvo que se
//!   fuerce). Solo lo acepta si la firma es buena **y** es más nuevo que el que ya tiene (`published`): así nadie
//!   puede devolver a Locall un catálogo antiguo aunque esté bien firmado.
//! - [`Nucleo::instalar_del_catalogo`] instala por nombre (`qwen3-4b`) y, **antes de descargar**, comprueba que lo que
//!   ofrece Hugging Face en ese commit coincide exactamente (rutas, tamaños y sha256) con lo firmado en el catálogo.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use locall_catalogo::{Catalogo, ErrorCatalogo, ModeloCatalogo, TAMANO_MAX, Variante};
use locall_registry::Modelo;

use crate::{ErrorNucleo, Nucleo, PeticionInstalar, ProgresoInstalacion, elegir_partes};

/// Dónde está el catálogo oficial (el archivo y, al lado, su firma `.minisig`).
pub const URL_CATALOGO: &str =
    "https://raw.githubusercontent.com/alvaritoo89/Locall/main/catalogo/catalogo.json";

/// Cada cuánto se mira, como mucho, si hay un catálogo nuevo.
pub const INTERVALO_COMPROBACION: Duration = Duration::from_secs(24 * 60 * 60);

/// Una firma de minisign ocupa unos 300 bytes: más de esto no es una firma.
const FIRMA_MAX: u64 = 4096;

/// De dónde salió el catálogo que se está usando.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrigenCatalogo {
    /// El que va dentro de este Locall.
    Incluido,
    /// Uno más nuevo, descargado de GitHub.
    Descargado,
}

/// El catálogo en uso y cuándo se miró por última vez si había otro.
#[derive(Debug, Clone)]
pub struct EstadoCatalogo {
    pub catalogo: Catalogo,
    pub origen: OrigenCatalogo,
    /// Segundos desde 1970 de la última vez que se miró en GitHub (`None`: nunca).
    pub comprobado: Option<u64>,
}

/// Lo que pasó al buscar un catálogo nuevo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actualizacion {
    /// Se miró hace menos de un día: no se ha vuelto a mirar.
    NoTocaba,
    /// Se miró y el que hay es igual o más nuevo.
    SinNovedades,
    /// Había uno nuevo con firma buena: ya se está usando.
    Actualizado,
}

const ARCHIVO: &str = "catalogo.json";
const FIRMA: &str = "catalogo.json.minisig";
const COMPROBADO: &str = "comprobado";

/// El catálogo con el que arranca el núcleo: el más nuevo entre el incluido y el guardado (si su firma es buena).
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
            // Solo pasaría si se compila Locall con un catálogo mal firmado (una prueba lo impide).
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
    /// El catálogo en uso.
    pub fn catalogo(&self) -> EstadoCatalogo {
        self.estado_catalogo().clone()
    }

    /// Mira en GitHub si hay un catálogo nuevo (como mucho una vez al día, salvo con `forzar`).
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
        // Se apunta que se ha mirado aunque falle, para no insistir cada vez si GitHub no responde.
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
        // Guardar primero (archivo temporal + renombrar): si se corta a medias, al abrir la firma no cuadra y se usa
        // el incluido; nunca uno a medias.
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

    /// Instala un modelo del catálogo por su nombre (`qwen3-4b` o `qwen3-4b:q8_0`).
    ///
    /// El identificador en Locall es el nombre del modelo para la variante de por defecto (`qwen3-4b`) y
    /// `nombre-variante` para las demás (`qwen3-4b-q8_0`).
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

    /// Lo que Hugging Face tiene en ese commit debe ser **exactamente** lo firmado en el catálogo: las mismas partes,
    /// con el mismo tamaño y el mismo sha256. Si no, no se descarga nada.
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

    /// ¿Cabría un modelo de `bytes` en este equipo? **Aproximado**: sin el archivo no se puede leer su cabecera, así
    /// que se estima como el archivo + un 10 % + 512 MB (contexto y motor). Devuelve la memoria estimada y el encaje.
    /// Como en [`Nucleo::evaluar`], la memoria del modelo cargado cuenta como disponible (se sustituiría).
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

    /// Descarga un archivo pequeño, sin pasar de `maximo` bytes.
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

/// Escribe en un archivo temporal y lo renombra encima del destino.
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
