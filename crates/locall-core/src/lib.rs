//! # locall-core
//!
//! El **núcleo** de Locall: une las piezas de la Fase 2 en un solo servicio, sin nada de HTTP (eso es `locall-api`,
//! regla 9). La API, la CLI y la app hablan con un [`Nucleo`].
//!
//! - [`Nucleo::abrir`]: lee la configuración y abre el almacén, el registro y el planificador.
//! - [`Nucleo::instalar`]: instala un modelo de Hugging Face fijado a un commit exacto, comprobando que es compatible
//!   **antes** de descargarlo (solo se lee su cabecera por la red).
//! - [`Nucleo::importar`]: incorpora un GGUF que ya está en el disco, sin copiarlo si se puede.
//! - [`Nucleo::modelos`], [`Nucleo::eliminar`], [`Nucleo::chat`], [`Nucleo::estado`].
//!
//! Todos los métodos toman `&self`: el núcleo se comparte entre hilos (`Arc<Nucleo>`).

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
/// Los tipos del catálogo (para los clientes del núcleo).
pub use locall_catalogo::{ArchivoCatalogo, Catalogo, ErrorCatalogo, ModeloCatalogo, Variante};

/// Resultados como mucho en una búsqueda en Hugging Face.
pub const BUSQUEDA_MAX: u32 = 50;
/// Longitud máxima del texto de una búsqueda (en caracteres).
pub const BUSQUEDA_TEXTO_MAX: usize = 100;

use huggingface::HuggingFace;
pub use huggingface::{ArchivoHf, HUGGING_FACE, InfoRevision, RepoEncontrado};
/// Los tipos de las conversaciones guardadas (para los clientes del núcleo).
pub use locall_registry::{Autor, Conversacion, MensajeGuardado};

// ─── Tipos públicos ───────────────────────────────────────────────────────────

/// Con qué abrir el núcleo.
#[derive(Debug, Clone)]
pub struct OpcionesNucleo {
    pub rutas: Rutas,
    /// Ruta de `llama-server.exe` (la pondrá el instalador; en desarrollo, la del paquete de la Fase 0).
    pub motor: PathBuf,
    /// Dirección de Hugging Face ([`HUGGING_FACE`]; otra solo en pruebas).
    pub hugging_face: String,
    /// Dirección del catálogo ([`URL_CATALOGO`]; otra solo en pruebas). La firma está al lado (`.minisig`).
    pub catalogo: String,
    /// Clave pública con la que se comprueba un catálogo descargado ([`locall_catalogo::CLAVE_PUBLICA`]; otra solo en
    /// pruebas). El catálogo que va dentro de Locall se comprueba siempre con la oficial.
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

/// Qué modelo instalar de Hugging Face.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeticionInstalar {
    /// `owner/nombre`, por ejemplo `Qwen/Qwen3-4B-GGUF`.
    pub repo: String,
    /// Rama, etiqueta o commit (`main` si no se sabe). Se fija al commit exacto.
    pub revision: String,
    /// El archivo GGUF (en un modelo dividido, cualquiera de sus partes).
    pub archivo: String,
    /// Identificador para Locall. `None` = se deduce del nombre del archivo.
    pub id: Option<String>,
}

/// El resultado de [`Nucleo::conversar`].
#[derive(Debug, Clone)]
pub struct RespuestaGuardada {
    /// La conversación, ya actualizada.
    pub conversacion: Conversacion,
    /// La respuesta guardada (`None` si se paró antes de que llegara nada).
    pub respuesta: Option<MensajeGuardado>,
    /// Las cifras de la respuesta.
    pub resumen: Resumen,
}

/// El turno de una conversación (ver `Nucleo::conversando`): al soltarlo, queda libre.
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

/// Convierte el «no encontrado» del registro en el error propio de las conversaciones.
fn no_encontrada(e: ErrorRegistro, id: i64) -> ErrorNucleo {
    match e {
        ErrorRegistro::NoEncontrado(_) => ErrorNucleo::ConversacionNoEncontrada(id),
        otro => ErrorNucleo::Registro(otro),
    }
}

/// Progreso de una instalación.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgresoInstalacion {
    /// Parte que se está descargando (1, 2…) de `partes`.
    pub parte: usize,
    pub partes: usize,
    /// Bytes descargados en total (todas las partes) de `total`.
    pub descargados: u64,
    pub total: u64,
}

// ─── El núcleo ────────────────────────────────────────────────────────────────

/// El núcleo de Locall.
pub struct Nucleo {
    rutas: Rutas,
    config: Config,
    avisos: Vec<String>,
    motor: PathBuf,
    almacen: Almacen,
    /// Con candado: SQLite solo admite un escritor y la conexión no se comparte entre hilos.
    registro: Mutex<Registro>,
    hf: HuggingFace,
    descargador: Descargador,
    planificador: Planificador<ArrancadorReal>,
    /// Identificadores que se están instalando ahora (dos instalaciones del mismo modelo a la vez se pisarían).
    instalando: Mutex<BTreeSet<String>>,
    /// Conversaciones que están recibiendo un mensaje ahora. Cada una atiende **un mensaje cada vez**: si llega otro
    /// (por ejemplo, justo después de parar una respuesta), espera a que el anterior termine de guardarse, para que
    /// los mensajes no se guarden desordenados.
    conversando: (Mutex<BTreeSet<i64>>, Condvar),
    /// Para que dos cambios de ajustes a la vez no se pisen al escribir `config.toml`.
    guardando_ajustes: Mutex<()>,
    /// El catálogo en uso (ver `catalogo.rs`).
    catalogo: Mutex<EstadoCatalogo>,
    url_catalogo: String,
    clave_catalogo: String,
    agente_catalogo: ureq::Agent,
}

impl Nucleo {
    /// Lee la configuración y abre todo. Crea las carpetas que falten.
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

    /// Avisos al abrir (ajustes desconocidos, almacén en OneDrive…), para mostrarlos al usuario.
    pub fn avisos(&self) -> &[String] {
        &self.avisos
    }

    /// Los ajustes que hay ahora en `config.toml` (pueden diferir de [`Nucleo::config`], los que se leyeron al
    /// abrir, si se han cambiado después: se aplican al volver a abrir Locall).
    pub fn ajustes_guardados(&self) -> Result<Config, ErrorNucleo> {
        Ok(Config::leer(&self.rutas.config())?.0)
    }

    /// Guarda estos cambios en `config.toml` (ver [`Config::guardar_cambios`]) y devuelve los ajustes guardados.
    /// Se aplican la próxima vez que se abra Locall.
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

    /// ¿Está trabajando? (instalando un modelo, cargándolo o respondiendo). Un modelo cargado sin usar no cuenta.
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

    /// Memoria que ocupa ahora el motor con el modelo cargado (`None` si no hay ninguno).
    pub fn memoria_del_modelo(&self) -> Option<u64> {
        let pid = self.planificador.con_motor(|motor| motor.pid())?;
        locall_system::memoria_de_proceso(pid).ok()
    }

    /// Cuánto falta para que el modelo cargado se descargue por inactividad (ver el planificador).
    pub fn se_descarga_en(&self) -> Option<std::time::Duration> {
        self.planificador.se_descarga_en()
    }

    /// ¿Cabe este modelo en el equipo, con el contexto de la configuración? (`locall-compat`)
    ///
    /// Cargar un modelo **sustituye** al que haya cargado, así que la memoria de ese cuenta como disponible: si no, la
    /// app diría «no cabe» de modelos que sí caben.
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

    /// Qué está haciendo el planificador (qué modelo está cargado).
    pub fn estado(&self) -> Estado {
        self.planificador.estado()
    }

    /// Los modelos instalados.
    pub fn modelos(&self) -> Result<Vec<Modelo>, ErrorNucleo> {
        Ok(self.registro().listar()?)
    }

    /// Un modelo instalado, o `None`.
    pub fn modelo(&self, id: &str) -> Result<Option<Modelo>, ErrorNucleo> {
        let Ok(id) = Segmento::nuevo(id) else {
            return Ok(None);
        };
        Ok(self.registro().obtener(&id)?)
    }

    // ─── Hugging Face ─────────────────────────────────────────────────────────

    /// Consulta un repositorio: su commit exacto, licencia y los GGUF que contiene.
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

    /// Busca en Hugging Face repositorios con modelos GGUF cuyo nombre contenga `texto`, los más descargados primero
    /// (como mucho [`BUSQUEDA_MAX`]). Solo se envía a Hugging Face el texto buscado.
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

    /// Instala un modelo de Hugging Face.
    ///
    /// Si se cancela (`cancelar`), lo descargado se conserva y la próxima vez continúa donde se quedó.
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

        // Comprobar la compatibilidad leyendo solo la cabecera, antes de descargar nada.
        let url_principal = self.hf.url_archivo(repo, &info.commit, &partes[0].ruta);
        let cabecera = self.hf.leer_cabecera(&url_principal)?;
        comprobar(&cabecera, self.config.contexto)?;

        // Espacio para lo que falte por descargar.
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
            verificado: false, // el catálogo verificado llegará más adelante
            archivos,
        };
        let modelo = self.registrar(nuevo)?;
        log::info!("modelo «{id}» instalado");
        Ok(modelo)
    }

    /// Incorpora un GGUF que ya está en el disco (en un modelo dividido, su primera parte).
    ///
    /// Si está en el mismo disco que el almacén, **no se copia** (enlace duro): no ocupa el doble. Al eliminar el
    /// modelo de Locall, el archivo original no se toca.
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
            // Enlace duro si se puede; si está en otro disco, copia (con su verificación de espacio).
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

    /// Elimina un modelo: su registro, su carpeta y los archivos del almacén que **ningún otro modelo** use.
    /// Devuelve los bytes liberados. Un archivo importado con enlace duro no se borra (es el original del usuario).
    pub fn eliminar(&self, id: &str) -> Result<u64, ErrorNucleo> {
        let seg = Segmento::nuevo(id).map_err(|_| ErrorNucleo::NoInstalado(id.to_owned()))?;
        let mut registro = self.registro();
        let modelo = registro
            .obtener(&seg)?
            .ok_or_else(|| ErrorNucleo::NoInstalado(id.to_owned()))?;

        // Si está cargado, descargarlo de la memoria (si alguien lo usa, no se puede eliminar ahora).
        let (es_este, en_uso) = match self.planificador.estado() {
            Estado::Cargando { modelo } => (modelo == id, true),
            Estado::Listo { modelo, en_uso } => (modelo == id, en_uso > 0),
            Estado::Vacio => (false, false),
        };
        if es_este && (en_uso || !self.planificador.descargar()) {
            return Err(ErrorNucleo::EnUso(id.to_owned()));
        }

        // Primero el registro (así el modelo deja de existir para todos) y después la carpeta y los archivos.
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
                // Importado con enlace duro (el original sigue siendo del usuario) o ya no estaba: nada que hacer.
                Err(ErrorAlmacen::BlobEnUso(_) | ErrorAlmacen::BlobNoEncontrado(_)) => {}
                Err(e) => log::warn!("no se pudo borrar un archivo de «{id}»: {e}"),
            }
        }
        log::info!("modelo «{id}» eliminado ({liberados} bytes liberados)");
        Ok(liberados)
    }

    // ─── Conversación ─────────────────────────────────────────────────────────

    /// Conversa con un modelo instalado. Lo carga si hace falta (esperando su turno; `cancelar` también sirve
    /// para irse de la cola) y entrega la respuesta por trozos.
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

    // ─── Conversaciones guardadas ─────────────────────────────────────────────

    /// Las conversaciones guardadas, de la más reciente a la más antigua.
    pub fn conversaciones(&self) -> Result<Vec<Conversacion>, ErrorNucleo> {
        Ok(self.registro().conversaciones()?)
    }

    /// Una conversación con sus mensajes.
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

    /// Crea una conversación vacía.
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

    /// Elimina una conversación de verdad (también del disco: ver `locall-registry`).
    pub fn eliminar_conversacion(&self, id: i64) -> Result<(), ErrorNucleo> {
        self.registro()
            .eliminar_conversacion(id)
            .map_err(|e| no_encontrada(e, id))?;
        log::info!("conversación {id} eliminada");
        Ok(())
    }

    /// Escribe `contenido` en la conversación `id` y guarda la respuesta del modelo.
    ///
    /// Se envía al modelo **toda la conversación guardada** (con las instrucciones del sistema). Si el modelo falla
    /// (por ejemplo, porque la conversación ya no cabe), la pregunta se quita, para que al reintentar no quede
    /// repetida. Si se para antes de que llegue nada, también.
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
        // 0) Que existan la conversación y el modelo (en ese orden), antes de guardar nada.
        if self.registro().conversacion(id)?.is_none() {
            return Err(ErrorNucleo::ConversacionNoEncontrada(id));
        }
        if self.modelo(modelo)?.is_none() {
            return Err(ErrorNucleo::NoInstalado(modelo.to_owned()));
        }
        // 1) Guardar la pregunta y los ajustes, y leer la conversación entera.
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

        // 2) Preguntar al modelo.
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

        // 3) Guardar la respuesta (también si se paró a medias, marcada como cortada).
        let mut registro = self.registro();
        let respuesta = if resumen.texto.is_empty() && resumen.razonamiento.is_empty() {
            registro.quitar_ultimo_mensaje(id)?; // se paró antes de que llegara nada
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
                    // (si se eliminó la conversación mientras respondía)
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

    /// Espera a que la conversación `id` no esté atendiendo otro mensaje y la marca como ocupada hasta que se suelte
    /// lo devuelto. Mientras espera, mira `cancelar`.
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

    /// Cómo arrancar el motor para un modelo instalado, con los ajustes del usuario.
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
                capas_gpu: 0, // la gráfica se activará cuando esté validada (D1)
                paralelo: self.config.paralelo,
                registro: self.rutas.registros().join("motor.log"),
            },
        })
    }

    // ─── Ayudantes ────────────────────────────────────────────────────────────

    fn registro(&self) -> MutexGuard<'_, Registro> {
        self.registro.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// El identificador pedido (o el deducido del archivo), validado y sin usar.
    fn id_libre(&self, pedido: Option<&str>, archivo: &str) -> Result<Segmento, ErrorNucleo> {
        let texto = pedido.map_or_else(|| nombres::id_desde_archivo(archivo), str::to_owned);
        let id = Segmento::nuevo(&texto).map_err(|_| ErrorNucleo::IdInvalido(texto.clone()))?;
        if self.registro().obtener(&id)?.is_some() {
            return Err(ErrorNucleo::YaInstalado(texto));
        }
        Ok(id)
    }

    /// Marca el identificador como "instalándose" hasta que se suelte el turno.
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

    /// Crea el snapshot y registra el modelo (si el registro falla, se deshace el snapshot).
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

/// Mientras exista, el identificador consta como "instalándose".
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

/// Comprueba la compatibilidad con el motor (los avisos no impiden instalar).
fn comprobar(cabecera: &Cabecera, contexto: u32) -> Result<(), ErrorNucleo> {
    let comprobacion = locall_compat::comprobar(cabecera, u64::from(contexto));
    if comprobacion.es_compatible() {
        Ok(())
    } else {
        Err(ErrorNucleo::Incompatible(comprobacion.problemas))
    }
}

/// Elige del repositorio el archivo pedido y, si es un modelo dividido, todas sus partes (en orden).
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

// ─── Errores ──────────────────────────────────────────────────────────────────

/// Lo que puede salir mal en el núcleo.
#[derive(Debug)]
pub enum ErrorNucleo {
    Config(ErrorConfig),
    /// Un ajuste que se quiso cambiar no es válido (o no se puede cambiar así).
    AjusteInvalido(String),
    /// Ese nombre no está en el catálogo.
    NoEnCatalogo(String),
    /// Lo que hay en Hugging Face no coincide con lo firmado en el catálogo.
    CatalogoNoCoincide(String),
    /// El catálogo descargado no sirve (firma, formato…).
    Catalogo(ErrorCatalogo),
    /// GitHub no ha dado el catálogo (código HTTP).
    CatalogoNoDisponible(u16),
    Almacen(ErrorAlmacen),
    Registro(ErrorRegistro),
    /// `owner/nombre` no válido.
    RepoInvalido(String),
    RevisionInvalida(String),
    /// El texto de una búsqueda está vacío, es demasiado largo o tiene caracteres de control.
    BusquedaInvalida(String),
    /// El identificador pedido no es válido para Locall.
    IdInvalido(String),
    /// Hugging Face no tiene ese repositorio (o es privado).
    RepoNoEncontrado(String),
    /// Hay que aceptar condiciones con una cuenta de Hugging Face (no se admite en la v1).
    Protegido(String),
    /// Hugging Face respondió con un error.
    HuggingFace {
        estado: u16,
    },
    /// Hugging Face respondió algo que no entendemos.
    RespuestaHf(String),
    /// Sin conexión, o se cortó.
    Red(String),
    /// El repositorio no tiene ese archivo.
    ArchivoNoEncontrado {
        archivo: String,
        disponibles: Vec<String>,
    },
    /// Falta una parte de un modelo dividido.
    FaltaParte(String),
    /// El archivo no trae huella sha256: no se podría verificar la descarga.
    SinHuella(String),
    Cabecera(ErrorGguf),
    /// El motor no puede usar este modelo.
    Incompatible(Vec<Problema>),
    YaInstalado(String),
    YaInstalandose(String),
    NoInstalado(String),
    /// No hay ninguna conversación con ese número.
    ConversacionNoEncontrada(i64),
    /// El modelo se está usando o cargando ahora mismo.
    EnUso(String),
    Descarga(ErrorDescarga),
    /// Cancelado a petición (una descarga a medias se conserva para continuar).
    Cancelado,
    /// El modelo no se pudo cargar.
    Carga(ErrorCarga),
    /// Fallo del motor durante la conversación.
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
