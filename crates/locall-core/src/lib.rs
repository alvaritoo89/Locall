//! # locall-core
//!
//! Locall's **core**: joins the Phase 2 pieces into a single service, without any HTTP (that is `locall-api`, rule 9).
//! The API, the CLI and the app talk to a [`Nucleo`].
//!
//! - [`Nucleo::abrir`]: reads the configuration and opens the store, the registry and the scheduler.
//! - [`Nucleo::instalar`]: installs a Hugging Face model pinned to an exact commit, checking that it is compatible
//!   **before** downloading it (only its header is read over the network).
//! - [`Nucleo::importar`]: brings in a GGUF that is already on disk, without copying it if possible.
//! - [`Nucleo::modelos`], [`Nucleo::eliminar`], [`Nucleo::chat`], [`Nucleo::estado`].
//!
//! All methods take `&self`: the core is shared between threads (`Arc<Nucleo>`).

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use locall_compat::{Evaluacion, Problema};
use locall_config::{Config, ErrorConfig, Rutas};
use locall_download::{Descargador, ErrorDescarga, Opciones as OpcionesDescarga, Pedido};
use locall_engine::{ConfigMotor, ErrorMotor, Evento, Fin, Mensaje, PeticionChat, Resumen, Rol};
use locall_gguf::{Cabecera, ErrorGguf};
use locall_registry::{ArchivoModelo, ErrorRegistro, MensajeNuevo, Modelo, ModeloNuevo, Registro};
use locall_scheduler::{
    ArrancadorReal, Carga, ErrorCarga, ErrorPlanificador, Estado, Opciones as OpcionesPlan,
    Planificador,
};
use locall_store::{Almacen, ErrorAlmacen, Huella, NombreArchivo, Segmento};

mod catalogo;
mod huggingface;
mod nombres;

pub use catalogo::{
    Actualizacion, EstadoCatalogo, INTERVALO_COMPROBACION, OrigenCatalogo, URL_CATALOGO,
};
/// The catalog types (for the core's clients).
pub use locall_catalogo::{ArchivoCatalogo, Catalogo, ErrorCatalogo, ModeloCatalogo, Variante};

/// Maximum results in a Hugging Face search.
pub const BUSQUEDA_MAX: u32 = 50;
/// Maximum length of a search text (in characters).
pub const BUSQUEDA_TEXTO_MAX: usize = 100;

use huggingface::HuggingFace;
pub use huggingface::{ArchivoHf, HUGGING_FACE, InfoRevision, RepoEncontrado};
/// The saved conversation types (for the core's clients).
pub use locall_registry::{Autor, Conversacion, MensajeGuardado};

// ─── Public types ─────────────────────────────────────────────────────────────

/// What to open the core with.
#[derive(Debug, Clone)]
pub struct OpcionesNucleo {
    pub rutas: Rutas,
    /// Path of `llama-server.exe` (the installer will set it; in development, the one from the Phase 0 package).
    pub motor: PathBuf,
    /// Hugging Face's address ([`HUGGING_FACE`]; another one only in tests).
    pub hugging_face: String,
    /// The catalog's address ([`URL_CATALOGO`]; another one only in tests). The signature is next to it (`.minisig`).
    pub catalogo: String,
    /// Public key used to check a downloaded catalog ([`locall_catalogo::CLAVE_PUBLICA`]; another one only in tests).
    /// The catalog shipped inside Locall is always checked with the official one.
    pub clave_catalogo: String,
}

impl OpcionesNucleo {
    pub fn nuevas(rutas: Rutas, motor: impl Into<PathBuf>) -> OpcionesNucleo {
        OpcionesNucleo {
            rutas,
            motor: motor.into(),
            hugging_face: HUGGING_FACE.to_owned(),
            catalogo: URL_CATALOGO.to_owned(),
            clave_catalogo: locall_catalogo::CLAVE_PUBLICA.to_owned(),
        }
    }
}

/// Which Hugging Face model to install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeticionInstalar {
    /// `owner/name`, for example `Qwen/Qwen3-4B-GGUF`.
    pub repo: String,
    /// Branch, tag or commit (`main` if unknown). It is pinned to the exact commit.
    pub revision: String,
    /// The GGUF file (in a split model, any of its parts).
    pub archivo: String,
    /// Identifier for Locall. `None` = deduced from the file name.
    pub id: Option<String>,
}

/// The result of [`Nucleo::conversar`].
#[derive(Debug, Clone)]
pub struct RespuestaGuardada {
    /// The conversation, already updated.
    pub conversacion: Conversacion,
    /// The saved answer (`None` if it was stopped before anything arrived).
    pub respuesta: Option<MensajeGuardado>,
    /// The answer's figures.
    pub resumen: Resumen,
}

/// A conversation's turn (see `Nucleo::conversando`): dropping it frees it.
struct TurnoConversacion<'a> {
    nucleo: &'a Nucleo,
    id: i64,
}

impl Drop for TurnoConversacion<'_> {
    fn drop(&mut self) {
        let (ocupadas, aviso) = &self.nucleo.conversando;
        ocupadas
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.id);
        aviso.notify_all();
    }
}

/// Converts the registry's «not found» into the conversations' own error.
fn no_encontrada(e: ErrorRegistro, id: i64) -> ErrorNucleo {
    match e {
        ErrorRegistro::NoEncontrado(_) => ErrorNucleo::ConversacionNoEncontrada(id),
        otro => ErrorNucleo::Registro(otro),
    }
}

/// Progress of an installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgresoInstalacion {
    /// Part being downloaded (1, 2…) of `partes`.
    pub parte: usize,
    pub partes: usize,
    /// Bytes downloaded in total (all parts) of `total`.
    pub descargados: u64,
    pub total: u64,
}

// ─── The core ─────────────────────────────────────────────────────────────────

/// Locall's core.
pub struct Nucleo {
    rutas: Rutas,
    config: Config,
    avisos: Vec<String>,
    motor: PathBuf,
    almacen: Almacen,
    /// Behind a lock: SQLite only allows one writer and the connection is not shared between threads.
    registro: Mutex<Registro>,
    hf: HuggingFace,
    descargador: Descargador,
    planificador: Planificador<ArrancadorReal>,
    /// Identifiers being installed right now (two installations of the same model at once would clash).
    instalando: Mutex<BTreeSet<String>>,
    /// Conversations receiving a message right now. Each one handles **one message at a time**: if another arrives (for
    /// example, right after stopping an answer), it waits for the previous one to finish saving, so messages are not
    /// saved out of order.
    conversando: (Mutex<BTreeSet<i64>>, Condvar),
    /// So two settings changes at once do not clash when writing `config.toml`.
    guardando_ajustes: Mutex<()>,
    /// The catalog in use (see `catalogo.rs`).
    catalogo: Mutex<EstadoCatalogo>,
    url_catalogo: String,
    clave_catalogo: String,
    agente_catalogo: ureq::Agent,
}

impl Nucleo {
    /// Reads the configuration and opens everything. Creates the missing folders.
    pub fn abrir(opciones: OpcionesNucleo) -> Result<Nucleo, ErrorNucleo> {
        let rutas = opciones.rutas;
        let rutas_catalogo = rutas.catalogo();
        let (config, avisos_config) = Config::leer(&rutas.config())?;
        let mut avisos: Vec<String> = avisos_config.iter().map(ToString::to_string).collect();

        let almacen = Almacen::abrir(config.carpeta_modelos(&rutas))?;
        if almacen.esta_en_onedrive() {
            avisos.push(
                "la carpeta de modelos está dentro de OneDrive: intentará subir gigas a la nube; mejor elige otra \
                 en config.toml (carpeta_modelos)"
                    .into(),
            );
        }
        let limpiados = almacen.limpiar_temporales()?;
        if limpiados > 0 {
            log::info!("{limpiados} archivos temporales abandonados borrados");
        }
        std::fs::create_dir_all(rutas.registros()).map_err(|error| {
            ErrorNucleo::Config(ErrorConfig::Io {
                ruta: rutas.registros(),
                error,
            })
        })?;
        let registro = Registro::abrir(&rutas.base_de_datos())?;
        let planificador = Planificador::nuevo(
            ArrancadorReal::default(),
            OpcionesPlan {
                inactividad: config.inactividad(),
            },
        );
        for aviso in &avisos {
            log::warn!("{aviso}");
        }
        log::info!("núcleo abierto en {}", rutas.datos().display());
        Ok(Nucleo {
            hf: HuggingFace::nuevo(&opciones.hugging_face),
            descargador: Descargador::nuevo(OpcionesDescarga::default()),
            rutas,
            config,
            avisos,
            motor: opciones.motor,
            almacen,
            registro: Mutex::new(registro),
            planificador,
            instalando: Mutex::new(BTreeSet::new()),
            conversando: (Mutex::new(BTreeSet::new()), Condvar::new()),
            guardando_ajustes: Mutex::new(()),
            catalogo: Mutex::new(catalogo::cargar(&rutas_catalogo, &opciones.clave_catalogo)),
            url_catalogo: opciones.catalogo,
            clave_catalogo: opciones.clave_catalogo,
            agente_catalogo: huggingface::agente_https(),
        })
    }

    /// Warnings on opening (unknown settings, store in OneDrive…), to show to the user.
    pub fn avisos(&self) -> &[String] {
        &self.avisos
    }

    /// The settings currently in `config.toml` (they may differ from [`Nucleo::config`], those read on opening, if they
    /// were changed afterwards: they take effect when Locall is reopened).
    pub fn ajustes_guardados(&self) -> Result<Config, ErrorNucleo> {
        Ok(Config::leer(&self.rutas.config())?.0)
    }

    /// Saves these changes to `config.toml` (see [`Config::guardar_cambios`]) and returns the saved settings.
    /// They take effect the next time Locall is opened.
    pub fn guardar_ajustes(
        &self,
        cambios: &[(&str, locall_config::lector::Valor)],
    ) -> Result<Config, ErrorNucleo> {
        let _candado = self
            .guardando_ajustes
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let guardada = Config::guardar_cambios(&self.rutas, cambios).map_err(|e| match e {
            ErrorConfig::Valor { clave, mensaje, .. } => {
                ErrorNucleo::AjusteInvalido(format!("«{clave}»: {mensaje}"))
            }
            ErrorConfig::NoEditable(_) => ErrorNucleo::AjusteInvalido(e.to_string()),
            otro => ErrorNucleo::Config(otro),
        })?;
        log::info!("ajustes guardados en config.toml (se aplican al volver a abrir Locall)");
        Ok(guardada)
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn rutas(&self) -> &Rutas {
        &self.rutas
    }

    /// Is it working? (installing a model, loading it or answering). A loaded model that is not in use does not count.
    pub fn ocupado(&self) -> bool {
        let instalando = !self
            .instalando
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty();
        instalando
            || match self.planificador.estado() {
                Estado::Cargando { .. } => true,
                Estado::Listo { en_uso, .. } => en_uso > 0,
                Estado::Vacio => false,
            }
    }

    /// Memory the engine with the loaded model takes right now (`None` if there is none).
    pub fn memoria_del_modelo(&self) -> Option<u64> {
        let pid = self.planificador.con_motor(|motor| motor.pid())?;
        locall_system::memoria_de_proceso(pid).ok()
    }

    /// How long until the loaded model unloads due to inactivity (see the scheduler).
    pub fn se_descarga_en(&self) -> Option<std::time::Duration> {
        self.planificador.se_descarga_en()
    }

    /// Does this model fit on the computer, with the configured context? (`locall-compat`)
    ///
    /// Loading a model **replaces** the one that is loaded, so that one's memory counts as available: otherwise the app
    /// would say «does not fit» for models that do fit.
    pub fn evaluar(&self, id: &str) -> Result<Evaluacion, ErrorNucleo> {
        let modelo = self
            .modelo(id)?
            .ok_or_else(|| ErrorNucleo::NoInstalado(id.to_owned()))?;
        let ruta = self.carga(id)?.config.modelo;
        let cabecera = locall_gguf::leer_archivo(&ruta).map_err(ErrorNucleo::Cabecera)?;
        let disponible = locall_system::memoria()
            .map(|m| m.disponible)
            .unwrap_or(0)
            .saturating_add(self.memoria_del_modelo().unwrap_or(0));
        Ok(locall_compat::evaluar(
            &cabecera,
            modelo.bytes_totales(),
            u64::from(self.config.contexto),
            disponible,
        ))
    }

    /// What the scheduler is doing (which model is loaded).
    pub fn estado(&self) -> Estado {
        self.planificador.estado()
    }

    /// The installed models.
    pub fn modelos(&self) -> Result<Vec<Modelo>, ErrorNucleo> {
        Ok(self.registro().listar()?)
    }

    /// An installed model, or `None`.
    pub fn modelo(&self, id: &str) -> Result<Option<Modelo>, ErrorNucleo> {
        let Ok(id) = Segmento::nuevo(id) else {
            return Ok(None);
        };
        Ok(self.registro().obtener(&id)?)
    }

    // ─── Hugging Face ─────────────────────────────────────────────────────────

    /// Queries a repository: its exact commit, license and the GGUFs it contains.
    pub fn consultar(
        &self,
        repo: &str,
        revision: &str,
    ) -> Result<(InfoRevision, Vec<ArchivoHf>), ErrorNucleo> {
        if !nombres::repo_valido(repo) {
            return Err(ErrorNucleo::RepoInvalido(repo.to_owned()));
        }
        if !nombres::revision_valida(revision) {
            return Err(ErrorNucleo::RevisionInvalida(revision.to_owned()));
        }
        let info = self.hf.revision(repo, revision)?;
        let ggufs = self
            .hf
            .archivos(repo, &info.commit)?
            .into_iter()
            .filter(|a| a.ruta.ends_with(".gguf"))
            .collect();
        Ok((info, ggufs))
    }

    /// Searches Hugging Face for repositories with GGUF models whose name contains `texto`, most downloaded first (at
    /// most [`BUSQUEDA_MAX`]). Only the searched text is sent to Hugging Face.
    pub fn buscar(&self, texto: &str, limite: u32) -> Result<Vec<RepoEncontrado>, ErrorNucleo> {
        let texto = texto.trim();
        if texto.is_empty()
            || texto.chars().count() > BUSQUEDA_TEXTO_MAX
            || texto.chars().any(char::is_control)
        {
            return Err(ErrorNucleo::BusquedaInvalida(
                texto.chars().take(BUSQUEDA_TEXTO_MAX).collect(),
            ));
        }
        self.hf.buscar(texto, limite.clamp(1, BUSQUEDA_MAX))
    }

    /// Installs a Hugging Face model.
    ///
    /// If it is cancelled (`cancelar`), what was downloaded is kept and next time it continues where it left off.
    pub fn instalar(
        &self,
        peticion: &PeticionInstalar,
        cancelar: &AtomicBool,
        mut progreso: impl FnMut(ProgresoInstalacion),
    ) -> Result<Modelo, ErrorNucleo> {
        let repo = peticion.repo.as_str();
        let (info, ggufs) = self.consultar(repo, &peticion.revision)?;
        if info.protegido {
            return Err(ErrorNucleo::Protegido(repo.to_owned()));
        }
        let partes = elegir_partes(&ggufs, &peticion.archivo)?;
        let huellas: Vec<Huella> = partes
            .iter()
            .map(|p| {
                p.huella
                    .clone()
                    .ok_or_else(|| ErrorNucleo::SinHuella(p.nombre().to_owned()))
            })
            .collect::<Result<_, _>>()?;
        let nombre_principal = partes[0].nombre().to_owned();
        let id = self.id_libre(peticion.id.as_deref(), &nombre_principal)?;
        let _turno = self.reservar(&id)?;

        // Check compatibility by reading only the header, before downloading anything.
        let url_principal = self.hf.url_archivo(repo, &info.commit, &partes[0].ruta);
        let cabecera = self.hf.leer_cabecera(&url_principal)?;
        comprobar(&cabecera, self.config.contexto)?;

        // Space for what is left to download.
        let total: u64 = partes.iter().map(|p| p.bytes).sum();
        let faltan: u64 = partes
            .iter()
            .zip(&huellas)
            .filter(|(_, h)| !self.almacen.contiene(h))
            .map(|(p, _)| p.bytes)
            .sum();
        self.almacen.comprobar_espacio(faltan)?;

        log::info!(
            "instalando «{id}» desde hf:{repo}@{} ({} partes, {total} bytes)",
            info.commit,
            partes.len()
        );
        let mut base = 0u64;
        for (i, (parte, huella)) in partes.iter().zip(&huellas).enumerate() {
            let pedido = Pedido {
                url: self.hf.url_archivo(repo, &info.commit, &parte.ruta),
                huella: huella.clone(),
                bytes: parte.bytes,
            };
            self.descargador
                .descargar(&self.almacen, &pedido, cancelar, |p| {
                    progreso(ProgresoInstalacion {
                        parte: i + 1,
                        partes: partes.len(),
                        descargados: base + p.descargados,
                        total,
                    });
                })
                .map_err(|e| match e {
                    ErrorDescarga::Cancelada => ErrorNucleo::Cancelado,
                    otro => ErrorNucleo::Descarga(otro),
                })?;
            base += parte.bytes;
        }

        let archivos = partes
            .iter()
            .zip(huellas)
            .map(|(p, huella)| {
                Ok(ArchivoModelo {
                    nombre: NombreArchivo::nuevo(p.nombre())?,
                    huella,
                    bytes: p.bytes,
                })
            })
            .collect::<Result<Vec<_>, ErrorAlmacen>>()?;
        let nuevo = ModeloNuevo {
            id: id.clone(),
            nombre: nombres::nombre_para_mostrar(&nombre_principal),
            origen: format!("hf:{repo}"),
            revision: info.commit.clone(),
            cuantizacion: nombres::cuantizacion(&nombre_principal),
            licencia: info.licencia.clone(),
            arquitectura: cabecera.resumen().arquitectura,
            contexto_max: cabecera.resumen().contexto_max,
            verificado: false, // the verified catalog will come later
            archivos,
        };
        let modelo = self.registrar(nuevo)?;
        log::info!("modelo «{id}» instalado");
        Ok(modelo)
    }

    /// Brings in a GGUF that is already on disk (in a split model, its first part).
    ///
    /// If it is on the same disk as the store, **it is not copied** (hard link): it does not take twice the space. When
    /// the model is removed from Locall, the original file is not touched.
    pub fn importar(&self, ruta: &Path, id: Option<&str>) -> Result<Modelo, ErrorNucleo> {
        let cabecera = locall_gguf::leer_archivo(ruta).map_err(ErrorNucleo::Cabecera)?;
        comprobar(&cabecera, self.config.contexto)?;
        let nombre_principal = ruta
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| ErrorNucleo::ArchivoNoEncontrado {
                archivo: ruta.display().to_string(),
                disponibles: Vec::new(),
            })?
            .to_owned();
        let id = self.id_libre(id, &nombre_principal)?;
        let _turno = self.reservar(&id)?;

        let mut rutas = vec![ruta.to_path_buf()];
        if let Some((base, _, n)) = nombres::parte_de(&nombre_principal) {
            rutas = (1..=n)
                .map(|i| ruta.with_file_name(nombres::nombre_de_parte(base, i, n)))
                .collect();
        }
        let mut archivos = Vec::new();
        for parte in &rutas {
            if !parte.is_file() {
                return Err(ErrorNucleo::FaltaParte(
                    parte
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                ));
            }
            // Hard link if possible; if it is on another disk, copy (with its space check).
            let huella = match self.almacen.adoptar_archivo(parte, None) {
                Ok(h) => h,
                Err(ErrorAlmacen::Io { .. }) => self.almacen.guardar_archivo(parte, None)?,
                Err(e) => return Err(e.into()),
            };
            let bytes =
                std::fs::metadata(parte)
                    .map(|m| m.len())
                    .map_err(|error| ErrorAlmacen::Io {
                        contexto: format!("consultar {}", parte.display()),
                        error,
                    })?;
            let nombre = parte.file_name().unwrap_or_default().to_string_lossy();
            archivos.push(ArchivoModelo {
                nombre: NombreArchivo::nuevo(&nombre)?,
                huella,
                bytes,
            });
        }
        let nuevo = ModeloNuevo {
            id: id.clone(),
            nombre: nombres::nombre_para_mostrar(&nombre_principal),
            origen: "local".into(),
            revision: "local".into(),
            cuantizacion: nombres::cuantizacion(&nombre_principal),
            licencia: None,
            arquitectura: cabecera.resumen().arquitectura,
            contexto_max: cabecera.resumen().contexto_max,
            verificado: false,
            archivos,
        };
        let modelo = self.registrar(nuevo)?;
        log::info!("modelo «{id}» importado del disco");
        Ok(modelo)
    }

    /// Removes a model: its registry entry, its folder and the store files that **no other model** uses.
    /// Returns the bytes freed. A file imported with a hard link is not deleted (it is the user's original).
    pub fn eliminar(&self, id: &str) -> Result<u64, ErrorNucleo> {
        let seg = Segmento::nuevo(id).map_err(|_| ErrorNucleo::NoInstalado(id.to_owned()))?;
        let mut registro = self.registro();
        let modelo = registro
            .obtener(&seg)?
            .ok_or_else(|| ErrorNucleo::NoInstalado(id.to_owned()))?;

        // If it is loaded, unload it from memory (if someone is using it, it cannot be removed now).
        let (es_este, en_uso) = match self.planificador.estado() {
            Estado::Cargando { modelo } => (modelo == id, true),
            Estado::Listo { modelo, en_uso } => (modelo == id, en_uso > 0),
            Estado::Vacio => (false, false),
        };
        if es_este && (en_uso || !self.planificador.descargar()) {
            return Err(ErrorNucleo::EnUso(id.to_owned()));
        }

        // The registry first (so the model stops existing for everyone) and then the folder and the files.
        let huellas = registro.eliminar(&seg)?;
        let en_uso = registro.huellas_en_uso()?;
        drop(registro);
        if let Ok(revision) = Segmento::nuevo(&modelo.revision)
            && let Err(e) = self.almacen.eliminar_snapshot(&seg, &revision)
        {
            log::warn!("no se pudo borrar la carpeta del modelo «{id}»: {e}");
        }
        let mut liberados = 0;
        for huella in huellas.iter().filter(|h| !en_uso.contains(*h)) {
            match self.almacen.eliminar_blob(huella) {
                Ok(bytes) => liberados += bytes,
                // Imported with a hard link (the original still belongs to the user) or it was already gone: nothing to do.
                Err(ErrorAlmacen::BlobEnUso(_) | ErrorAlmacen::BlobNoEncontrado(_)) => {}
                Err(e) => log::warn!("no se pudo borrar un archivo de «{id}»: {e}"),
            }
        }
        log::info!("modelo «{id}» eliminado ({liberados} bytes liberados)");
        Ok(liberados)
    }

    // ─── Conversation ─────────────────────────────────────────────────────────

    /// Chats with an installed model. Loads it if needed (waiting for its turn; `cancelar` also leaves the queue) and
    /// delivers the answer in chunks.
    pub fn chat(
        &self,
        id: &str,
        peticion: &PeticionChat,
        cancelar: &AtomicBool,
        al_recibir: impl FnMut(Evento<'_>),
    ) -> Result<Resumen, ErrorNucleo> {
        let carga = self.carga(id)?;
        let prestamo = self
            .planificador
            .usar_con(&carga, cancelar)
            .map_err(|e| match e {
                ErrorPlanificador::Cancelado => ErrorNucleo::Cancelado,
                ErrorPlanificador::Motor(e) => ErrorNucleo::Carga(e),
            })?;
        if let Ok(seg) = Segmento::nuevo(id)
            && let Err(e) = self.registro().marcar_uso(&seg, locall_registry::ahora())
        {
            log::warn!("no se pudo anotar el uso de «{id}»: {e}");
        }
        Ok(prestamo.chat(peticion, cancelar, al_recibir)?)
    }

    // ─── Saved conversations ──────────────────────────────────────────────────

    /// The saved conversations, from the most recent to the oldest.
    pub fn conversaciones(&self) -> Result<Vec<Conversacion>, ErrorNucleo> {
        Ok(self.registro().conversaciones()?)
    }

    /// A conversation with its messages.
    pub fn conversacion(
        &self,
        id: i64,
    ) -> Result<(Conversacion, Vec<MensajeGuardado>), ErrorNucleo> {
        let registro = self.registro();
        let c = registro
            .conversacion(id)?
            .ok_or(ErrorNucleo::ConversacionNoEncontrada(id))?;
        Ok((c, registro.mensajes(id)?))
    }

    /// Creates an empty conversation.
    pub fn crear_conversacion(&self, titulo: &str) -> Result<Conversacion, ErrorNucleo> {
        let titulo = if titulo.trim().is_empty() {
            "Nueva conversación"
        } else {
            titulo
        };
        Ok(self
            .registro()
            .crear_conversacion(titulo, locall_registry::ahora())?)
    }

    pub fn renombrar_conversacion(&self, id: i64, titulo: &str) -> Result<(), ErrorNucleo> {
        self.registro()
            .renombrar_conversacion(id, titulo)
            .map_err(|e| no_encontrada(e, id))
    }

    /// Deletes a conversation for real (also from disk: see `locall-registry`).
    pub fn eliminar_conversacion(&self, id: i64) -> Result<(), ErrorNucleo> {
        self.registro()
            .eliminar_conversacion(id)
            .map_err(|e| no_encontrada(e, id))?;
        log::info!("conversación {id} eliminada");
        Ok(())
    }

    /// Writes `contenido` to conversation `id` and saves the model's answer.
    ///
    /// The **whole saved conversation** is sent to the model (with the system instructions). If the model fails (for
    /// example, because the conversation no longer fits), the question is removed, so it is not duplicated on retry. The
    /// same if it is stopped before anything arrives.
    #[allow(clippy::too_many_arguments)]
    pub fn conversar(
        &self,
        id: i64,
        contenido: &str,
        modelo: &str,
        sistema: &str,
        temperatura: Option<f32>,
        cancelar: &AtomicBool,
        al_recibir: impl FnMut(Evento<'_>),
    ) -> Result<RespuestaGuardada, ErrorNucleo> {
        let _turno = self.turno_de_conversacion(id, cancelar)?;
        // 0) The conversation and the model must exist (in that order), before saving anything.
        if self.registro().conversacion(id)?.is_none() {
            return Err(ErrorNucleo::ConversacionNoEncontrada(id));
        }
        if self.modelo(modelo)?.is_none() {
            return Err(ErrorNucleo::NoInstalado(modelo.to_owned()));
        }
        // 1) Save the question and the settings, and read the whole conversation.
        let historial = {
            let mut registro = self.registro();
            registro
                .ajustar_conversacion(id, modelo, sistema, temperatura.map(f64::from))
                .map_err(|e| no_encontrada(e, id))?;
            registro.anadir_mensaje(
                id,
                &MensajeNuevo::del_usuario(contenido),
                locall_registry::ahora(),
            )?;
            registro.mensajes(id)?
        };

        // 2) Ask the model.
        let mut mensajes = Vec::with_capacity(historial.len() + 1);
        if !sistema.trim().is_empty() {
            mensajes.push(Mensaje::nuevo(Rol::Sistema, sistema.trim()));
        }
        for m in &historial {
            let rol = match m.autor {
                Autor::Usuario => Rol::Usuario,
                Autor::Asistente => Rol::Asistente,
            };
            mensajes.push(Mensaje::nuevo(rol, m.contenido.clone()));
        }
        let peticion = PeticionChat {
            mensajes,
            max_tokens: None,
            temperatura,
            semilla: None,
        };
        let resumen = match self.chat(modelo, &peticion, cancelar, al_recibir) {
            Ok(r) => r,
            Err(e) => {
                let _ = self.registro().quitar_ultimo_mensaje(id);
                return Err(e);
            }
        };

        // 3) Save the answer (also if it was stopped halfway, marked as cut).
        let mut registro = self.registro();
        let respuesta = if resumen.texto.is_empty() && resumen.razonamiento.is_empty() {
            registro.quitar_ultimo_mensaje(id)?; // it was stopped before anything arrived
            None
        } else {
            Some(
                registro
                    .anadir_mensaje(
                        id,
                        &MensajeNuevo {
                            autor: Autor::Asistente,
                            contenido: resumen.texto.clone(),
                            razonamiento: resumen.razonamiento.clone(),
                            modelo: Some(modelo.to_owned()),
                            tokens: Some(resumen.tokens_generados),
                            velocidad: resumen.escritura_por_segundo,
                            cortada: resumen.fin == Fin::Cancelado,
                        },
                        locall_registry::ahora(),
                    )
                    // (if the conversation was deleted while it was answering)
                    .map_err(|e| no_encontrada(e, id))?,
            )
        };
        let conversacion = registro
            .conversacion(id)?
            .ok_or(ErrorNucleo::ConversacionNoEncontrada(id))?;
        Ok(RespuestaGuardada {
            conversacion,
            respuesta,
            resumen,
        })
    }

    /// Waits until conversation `id` is not handling another message and marks it as busy until the returned value is
    /// dropped. While waiting, it checks `cancelar`.
    fn turno_de_conversacion<'a>(
        &'a self,
        id: i64,
        cancelar: &AtomicBool,
    ) -> Result<TurnoConversacion<'a>, ErrorNucleo> {
        let (ocupadas, aviso) = &self.conversando;
        let mut lista = ocupadas.lock().unwrap_or_else(PoisonError::into_inner);
        while lista.contains(&id) {
            if cancelar.load(Ordering::Relaxed) {
                return Err(ErrorNucleo::Cancelado);
            }
            lista = aviso
                .wait_timeout(lista, Duration::from_millis(100))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        lista.insert(id);
        Ok(TurnoConversacion { nucleo: self, id })
    }

    /// How to start the engine for an installed model, with the user's settings.
    pub fn carga(&self, id: &str) -> Result<Carga, ErrorNucleo> {
        let modelo = self
            .modelo(id)?
            .ok_or_else(|| ErrorNucleo::NoInstalado(id.to_owned()))?;
        let principal = modelo
            .archivos
            .first()
            .ok_or_else(|| ErrorNucleo::NoInstalado(id.to_owned()))?;
        let carpeta = self
            .almacen
            .carpeta_snapshot(&modelo.id, &Segmento::nuevo(&modelo.revision)?);
        Ok(Carga {
            modelo: modelo.id.to_string(),
            config: ConfigMotor {
                ejecutable: self.motor.clone(),
                modelo: carpeta.join(principal.nombre.to_string()),
                contexto: self.config.contexto,
                hilos: self.config.hilos_para_motor(),
                capas_gpu: 0, // the GPU will be enabled once it is validated (D1)
                paralelo: self.config.paralelo,
                registro: self.rutas.registros().join("motor.log"),
            },
        })
    }

    // ─── Helpers ──────────────────────────────────────────────────────────────

    fn registro(&self) -> MutexGuard<'_, Registro> {
        self.registro.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The requested identifier (or the one deduced from the file), validated and unused.
    fn id_libre(&self, pedido: Option<&str>, archivo: &str) -> Result<Segmento, ErrorNucleo> {
        let texto = pedido.map_or_else(|| nombres::id_desde_archivo(archivo), str::to_owned);
        let id = Segmento::nuevo(&texto).map_err(|_| ErrorNucleo::IdInvalido(texto.clone()))?;
        if self.registro().obtener(&id)?.is_some() {
            return Err(ErrorNucleo::YaInstalado(texto));
        }
        Ok(id)
    }

    /// Marks the identifier as "installing" until the turn is released.
    fn reservar(&self, id: &Segmento) -> Result<Turno<'_>, ErrorNucleo> {
        let mut instalando = self
            .instalando
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !instalando.insert(id.to_string()) {
            return Err(ErrorNucleo::YaInstalandose(id.to_string()));
        }
        Ok(Turno {
            instalando: &self.instalando,
            id: id.to_string(),
        })
    }

    /// Creates the snapshot and registers the model (if registering fails, the snapshot is undone).
    fn registrar(&self, nuevo: ModeloNuevo) -> Result<Modelo, ErrorNucleo> {
        let revision = Segmento::nuevo(&nuevo.revision)?;
        let enlaces: Vec<(NombreArchivo, Huella)> = nuevo
            .archivos
            .iter()
            .map(|a| (a.nombre.clone(), a.huella.clone()))
            .collect();
        self.almacen
            .crear_snapshot(&nuevo.id, &revision, &enlaces)?;
        let mut registro = self.registro();
        if let Err(e) = registro.registrar(&nuevo, locall_registry::ahora()) {
            drop(registro);
            let _ = self.almacen.eliminar_snapshot(&nuevo.id, &revision);
            return Err(e.into());
        }
        registro
            .obtener(&nuevo.id)?
            .ok_or_else(|| ErrorNucleo::NoInstalado(nuevo.id.to_string()))
    }
}

impl fmt::Debug for Nucleo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Nucleo")
            .field("datos", &self.rutas.datos())
            .field("estado", &self.estado())
            .finish_non_exhaustive()
    }
}

/// While it exists, the identifier is listed as "installing".
struct Turno<'a> {
    instalando: &'a Mutex<BTreeSet<String>>,
    id: String,
}

impl Drop for Turno<'_> {
    fn drop(&mut self) {
        self.instalando
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.id);
    }
}

/// Checks compatibility with the engine (warnings do not prevent installing).
fn comprobar(cabecera: &Cabecera, contexto: u32) -> Result<(), ErrorNucleo> {
    let comprobacion = locall_compat::comprobar(cabecera, u64::from(contexto));
    if comprobacion.es_compatible() {
        Ok(())
    } else {
        Err(ErrorNucleo::Incompatible(comprobacion.problemas))
    }
}

/// Picks the requested file from the repository and, if it is a split model, all its parts (in order).
fn elegir_partes(ggufs: &[ArchivoHf], pedido: &str) -> Result<Vec<ArchivoHf>, ErrorNucleo> {
    let elegido = ggufs
        .iter()
        .find(|a| a.ruta == pedido)
        .or_else(|| ggufs.iter().find(|a| a.nombre() == pedido))
        .ok_or_else(|| ErrorNucleo::ArchivoNoEncontrado {
            archivo: pedido.to_owned(),
            disponibles: ggufs.iter().map(|a| a.ruta.clone()).collect(),
        })?;
    let Some((base, _, n)) = nombres::parte_de(elegido.nombre()) else {
        return Ok(vec![elegido.clone()]);
    };
    let carpeta = elegido
        .ruta
        .rsplit_once('/')
        .map_or(String::new(), |(c, _)| format!("{c}/"));
    (1..=n)
        .map(|i| {
            let ruta = format!("{carpeta}{}", nombres::nombre_de_parte(base, i, n));
            ggufs
                .iter()
                .find(|a| a.ruta == ruta)
                .cloned()
                .ok_or_else(|| ErrorNucleo::FaltaParte(ruta))
        })
        .collect()
}

// ─── Errors ───────────────────────────────────────────────────────────────────

/// What can go wrong in the core.
#[derive(Debug)]
pub enum ErrorNucleo {
    Config(ErrorConfig),
    /// A setting someone tried to change is not valid (or cannot be changed this way).
    AjusteInvalido(String),
    /// That name is not in the catalog.
    NoEnCatalogo(String),
    /// What is on Hugging Face does not match what is signed in the catalog.
    CatalogoNoCoincide(String),
    /// The downloaded catalog is not usable (signature, format…).
    Catalogo(ErrorCatalogo),
    /// GitHub did not provide the catalog (HTTP code).
    CatalogoNoDisponible(u16),
    Almacen(ErrorAlmacen),
    Registro(ErrorRegistro),
    /// Invalid `owner/name`.
    RepoInvalido(String),
    RevisionInvalida(String),
    /// The search text is empty, too long or has control characters.
    BusquedaInvalida(String),
    /// The requested identifier is not valid for Locall.
    IdInvalido(String),
    /// Hugging Face does not have that repository (or it is private).
    RepoNoEncontrado(String),
    /// Conditions must be accepted with a Hugging Face account (not supported in v1).
    Protegido(String),
    /// Hugging Face answered with an error.
    HuggingFace {
        estado: u16,
    },
    /// Hugging Face answered something we do not understand.
    RespuestaHf(String),
    /// No connection, or it dropped.
    Red(String),
    /// The repository does not have that file.
    ArchivoNoEncontrado {
        archivo: String,
        disponibles: Vec<String>,
    },
    /// A part of a split model is missing.
    FaltaParte(String),
    /// The file has no sha256 hash: the download could not be verified.
    SinHuella(String),
    Cabecera(ErrorGguf),
    /// The engine cannot use this model.
    Incompatible(Vec<Problema>),
    YaInstalado(String),
    YaInstalandose(String),
    NoInstalado(String),
    /// There is no conversation with that number.
    ConversacionNoEncontrada(i64),
    /// The model is being used or loaded right now.
    EnUso(String),
    Descarga(ErrorDescarga),
    /// Cancelled on request (a half-finished download is kept to continue).
    Cancelado,
    /// The model could not be loaded.
    Carga(ErrorCarga),
    /// Engine failure during the conversation.
    Motor(ErrorMotor),
}

impl fmt::Display for ErrorNucleo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorNucleo::Config(e) => write!(f, "{e}"),
            ErrorNucleo::AjusteInvalido(m) => write!(f, "{m}"),
            ErrorNucleo::NoEnCatalogo(n) => write!(
                f,
                "«{n}» no está en el catálogo de Locall; búscalo en Hugging Face con propietario/nombre"
            ),
            ErrorNucleo::CatalogoNoCoincide(m) => write!(
                f,
                "lo que ofrece Hugging Face no coincide con el catálogo firmado ({m}); no se descarga nada"
            ),
            ErrorNucleo::Catalogo(e) => write!(f, "{e}"),
            ErrorNucleo::CatalogoNoDisponible(estado) => {
                write!(
                    f,
                    "no se pudo descargar el catálogo (GitHub respondió {estado})"
                )
            }
            ErrorNucleo::Almacen(e) => write!(f, "{e}"),
            ErrorNucleo::Registro(e) => write!(f, "{e}"),
            ErrorNucleo::RepoInvalido(r) => {
                write!(
                    f,
                    "«{r}» no es un repositorio válido; se escribe así: propietario/nombre"
                )
            }
            ErrorNucleo::RevisionInvalida(r) => write!(f, "«{r}» no es una revisión válida"),
            ErrorNucleo::BusquedaInvalida(_) => write!(
                f,
                "escribe qué buscar (de 1 a {BUSQUEDA_TEXTO_MAX} caracteres, en una línea)"
            ),
            ErrorNucleo::IdInvalido(id) => write!(
                f,
                "«{id}» no sirve como identificador: solo minúsculas, números, «-», «_» y «.»"
            ),
            ErrorNucleo::RepoNoEncontrado(r) => {
                write!(
                    f,
                    "Hugging Face no tiene el repositorio «{r}» (o es privado)"
                )
            }
            ErrorNucleo::Protegido(r) => write!(
                f,
                "«{r}» exige aceptar condiciones con una cuenta de Hugging Face; Locall todavía no lo admite"
            ),
            ErrorNucleo::HuggingFace { estado } => {
                write!(
                    f,
                    "Hugging Face respondió con el error {estado}; prueba más tarde"
                )
            }
            ErrorNucleo::RespuestaHf(e) => write!(f, "respuesta de Hugging Face no válida: {e}"),
            ErrorNucleo::Red(e) => write!(f, "no se pudo conectar con Hugging Face: {e}"),
            ErrorNucleo::ArchivoNoEncontrado {
                archivo,
                disponibles,
            } => {
                write!(f, "el repositorio no tiene «{archivo}»")?;
                if !disponibles.is_empty() {
                    write!(f, "; tiene: {}", disponibles.join(", "))?;
                }
                Ok(())
            }
            ErrorNucleo::FaltaParte(p) => write!(f, "falta la parte «{p}» del modelo"),
            ErrorNucleo::SinHuella(a) => write!(
                f,
                "«{a}» no tiene huella sha256 en Hugging Face: no se podría verificar, así que no se descarga"
            ),
            ErrorNucleo::Cabecera(_) => write!(f, "el archivo no es un modelo GGUF válido"),
            ErrorNucleo::Incompatible(problemas) => {
                write!(f, "este modelo no se puede usar: ")?;
                for (i, p) in problemas.iter().enumerate() {
                    if i > 0 {
                        write!(f, "; ")?;
                    }
                    write!(f, "{p}")?;
                }
                Ok(())
            }
            ErrorNucleo::YaInstalado(id) => write!(f, "ya hay un modelo instalado como «{id}»"),
            ErrorNucleo::YaInstalandose(id) => write!(f, "«{id}» ya se está instalando"),
            ErrorNucleo::NoInstalado(id) => write!(f, "no hay ningún modelo instalado como «{id}»"),
            ErrorNucleo::ConversacionNoEncontrada(id) => {
                write!(f, "no existe la conversación {id}")
            }
            ErrorNucleo::EnUso(id) => write!(
                f,
                "«{id}» se está usando ahora mismo; inténtalo cuando termine"
            ),
            ErrorNucleo::Descarga(e) => write!(f, "{e}"),
            ErrorNucleo::Cancelado => write!(f, "cancelado"),
            ErrorNucleo::Carga(e) => write!(f, "{e}"),
            ErrorNucleo::Motor(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ErrorNucleo {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ErrorNucleo::Config(e) => Some(e),
            ErrorNucleo::Almacen(e) => Some(e),
            ErrorNucleo::Registro(e) => Some(e),
            ErrorNucleo::Cabecera(e) => Some(e),
            ErrorNucleo::Descarga(e) => Some(e),
            ErrorNucleo::Carga(e) => Some(e),
            ErrorNucleo::Motor(e) => Some(e),
            _ => None,
        }
    }
}

impl From<ErrorConfig> for ErrorNucleo {
    fn from(e: ErrorConfig) -> Self {
        ErrorNucleo::Config(e)
    }
}

impl From<ErrorAlmacen> for ErrorNucleo {
    fn from(e: ErrorAlmacen) -> Self {
        ErrorNucleo::Almacen(e)
    }
}

impl From<ErrorRegistro> for ErrorNucleo {
    fn from(e: ErrorRegistro) -> Self {
        ErrorNucleo::Registro(e)
    }
}

impl From<ErrorMotor> for ErrorNucleo {
    fn from(e: ErrorMotor) -> Self {
        ErrorNucleo::Motor(e)
    }
}
