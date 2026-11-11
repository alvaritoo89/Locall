//! # locall-scheduler
//!
//! The scheduler decides **which model is loaded**. The app, the API and the CLI do not talk to the engine directly:
//! they ask the scheduler "I want to use this model" and receive a [`Prestamo`] (a loan).
//!
//! Rules:
//!
//! 1. **One model at a time** (it fits on 16 GB computers; Phase 0 decision). If the same model is requested with the
//!    same configuration, it is reused; if another one is requested, the current one is stopped and the new one
//!    started.
//! 2. While someone holds a **loan**, the model is not changed: whoever asks for another model **waits**.
//!    Several can hold a loan of the *same* model at once.
//! 3. If the model has gone [`Opciones::inactividad`] without use and nobody holds it, **it unloads by itself** to
//!    free the RAM.
//! 4. **First-come, first-served queue:** whoever arrived first is served first. Whoever asks for the already loaded
//!    model does not jump ahead of someone waiting for another model (otherwise that one could wait forever). The
//!    wait for a turn can be **cancelled**.
//!
//! The scheduler does not know `llama-server`: it works with any [`Arrancador`]. Locall uses [`ArrancadorReal`] (the
//! `locall-engine` engine); tests use a fake one, fast and exact.

use std::collections::VecDeque;
use std::fmt;
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use locall_engine::{ConfigMotor, Motor};

mod comprobacion;
#[cfg(test)]
mod pruebas;

pub use comprobacion::{ErrorCarga, evaluar_carga};

// ─── The Arrancador trait ─────────────────────────────────────────────────────

/// What the scheduler needs from an engine: knowing how to **start it** with a load.
///
/// To **stop it**, dropping it is enough (`Drop`), as [`locall_engine::Motor`] already does.
///
/// `Send + Sync + 'static` means: it can be shared between threads and borrows nothing from outside. The scheduler
/// needs it because it uses it from several threads and from its watcher.
pub trait Arrancador: Send + Sync + 'static {
    /// The running engine it returns.
    type Motor: Send + Sync + 'static;
    /// The error if it cannot start.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Starts an engine with `carga` and waits until it is ready.
    fn arrancar(&self, carga: &Carga) -> Result<Self::Motor, Self::Error>;

    /// Is this engine still alive? If its process exited by itself (out of memory, a crash…), the scheduler removes it
    /// and, if that model is requested again, loads it anew. By default, always alive.
    fn sigue_vivo(&self, motor: &Self::Motor) -> bool {
        let _ = motor;
        true
    }
}

/// The real starter: `llama-server` through `locall-engine`.
///
/// Before launching the engine it checks (part 9c) that the model is compatible and that it **fits in the available
/// memory**; if not, it returns the error instantly, without starting anything.
#[derive(Debug, Clone)]
pub struct ArrancadorReal {
    /// Maximum time to load a model.
    pub espera_maxima: Duration,
    /// Check compatibility and memory before loading. Only disabled if the user insists on trying (the estimate is
    /// conservative and can be slightly wrong).
    pub comprobar_antes: bool,
}

impl Default for ArrancadorReal {
    fn default() -> Self {
        ArrancadorReal {
            espera_maxima: Duration::from_secs(300),
            comprobar_antes: true,
        }
    }
}

impl Arrancador for ArrancadorReal {
    type Motor = Motor;
    type Error = ErrorCarga;

    fn arrancar(&self, carga: &Carga) -> Result<Motor, ErrorCarga> {
        if self.comprobar_antes {
            // The scheduler has already stopped the previous model: the memory it frees counts as available.
            let disponibles = locall_system::memoria()
                .map_err(ErrorCarga::Sistema)?
                .disponible;
            let evaluacion = comprobacion::evaluar_con_memoria(carga, disponibles)?;
            comprobacion::decidir(&evaluacion, disponibles)?;
        }
        let (motor, _tiempo) =
            Motor::arrancar_listo(&carga.config, self.espera_maxima, &AtomicBool::new(false))?;
        Ok(motor)
    }

    fn sigue_vivo(&self, motor: &Motor) -> bool {
        let vivo = motor.sigue_vivo();
        if !vivo {
            // The engine leaves written why it exited (out of memory, damaged model…): it is recorded in the log.
            let causa = motor.causa_del_cierre();
            log::warn!("el motor se ha cerrado solo (causa: {causa:?})");
        }
        vivo
    }
}

// ─── Public types ─────────────────────────────────────────────────────────────

/// Which model to load and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Carga {
    /// Model name for display (for example, `qwen2.5-1.5b`). Never a path.
    pub modelo: String,
    /// How to start the engine. If anything changes (the context, the layers on the GPU…), it is a different load.
    pub config: ConfigMotor,
}

/// Scheduler settings.
#[derive(Debug, Clone)]
pub struct Opciones {
    /// Idle time after which the model unloads by itself. `None` = it never unloads by itself.
    pub inactividad: Option<Duration>,
}

impl Default for Opciones {
    fn default() -> Self {
        Opciones {
            inactividad: Some(Duration::from_secs(5 * 60)),
        }
    }
}

/// What the scheduler is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Estado {
    /// No model is loaded.
    Vacio,
    /// This model is being loaded.
    Cargando { modelo: String },
    /// This model is ready. `en_uso` = active loans.
    Listo { modelo: String, en_uso: usize },
}

/// What can go wrong when requesting a model.
#[derive(Debug)]
pub enum ErrorPlanificador<E> {
    /// It was cancelled while waiting for its turn.
    Cancelado,
    /// The engine could not start.
    Motor(E),
}

impl<E: fmt::Display> fmt::Display for ErrorPlanificador<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorPlanificador::Cancelado => {
                write!(f, "petición cancelada mientras esperaba su turno")
            }
            ErrorPlanificador::Motor(e) => write!(f, "{e}"),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for ErrorPlanificador<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ErrorPlanificador::Cancelado => None,
            ErrorPlanificador::Motor(e) => Some(e),
        }
    }
}

/// How often the waiting request checks whether it has been cancelled.
const MIRAR_CANCELACION: Duration = Duration::from_millis(50);

// ─── The scheduler ────────────────────────────────────────────────────────────

/// The scheduler. Shared between threads with a reference (`&Planificador`) or an `Arc<Planificador>`.
pub struct Planificador<A: Arrancador> {
    compartido: Arc<Compartido<A>>,
    vigilante: Option<JoinHandle<()>>,
}

/// What the scheduler, the loans and the watcher share.
struct Compartido<A: Arrancador> {
    arrancador: A,
    opciones: Opciones,
    estado: Mutex<Interno<A::Motor>>,
    /// Notifies the waiters every time the state changes.
    cambio: Condvar,
}

/// The state, protected by the `Mutex`: only one thread at a time can read or change it.
struct Interno<M> {
    actual: Option<Cargado<M>>,
    /// Model some thread is loading right now (the others wait).
    cargando: Option<String>,
    /// Active loans of the current model.
    prestamos: usize,
    /// Last time a loan was released (or the model was loaded).
    ultimo_uso: Instant,
    /// The scheduler is shutting down: the watcher must finish.
    cerrando: bool,
    /// Turns of the waiting requests, in order of arrival.
    cola: VecDeque<u64>,
    /// Number of the next turn handed out.
    siguiente_turno: u64,
}

struct Cargado<M> {
    carga: Carga,
    motor: Arc<M>,
}

impl<A: Arrancador> Planificador<A> {
    pub fn nuevo(arrancador: A, opciones: Opciones) -> Planificador<A> {
        let compartido = Arc::new(Compartido {
            arrancador,
            opciones,
            estado: Mutex::new(Interno {
                actual: None,
                cargando: None,
                prestamos: 0,
                ultimo_uso: Instant::now(),
                cerrando: false,
                cola: VecDeque::new(),
                siguiente_turno: 0,
            }),
            cambio: Condvar::new(),
        });
        let para_vigilante = Arc::clone(&compartido);
        let vigilante = thread::Builder::new()
            .name("locall-vigilante".into())
            .spawn(move || vigilar(&para_vigilante))
            .expect("no se pudo crear el hilo vigilante");
        Planificador {
            compartido,
            vigilante: Some(vigilante),
        }
    }

    /// Requests to use the model in `carga`. If needed, stops the current model and loads this one.
    ///
    /// **Blocks** until it is its turn. To be able to cancel the wait, use [`Planificador::usar_con`].
    pub fn usar(&self, carga: &Carga) -> Result<Prestamo<A>, ErrorPlanificador<A::Error>> {
        self.usar_con(carga, &AtomicBool::new(false))
    }

    /// Like [`Planificador::usar`], but if another thread sets `cancelar` to `true` while it waits for its turn, it
    /// leaves the queue with [`ErrorPlanificador::Cancelado`]. (A load already started is not interrupted.)
    pub fn usar_con(
        &self,
        carga: &Carga,
        cancelar: &AtomicBool,
    ) -> Result<Prestamo<A>, ErrorPlanificador<A::Error>> {
        let c = &self.compartido;
        let mut estado = c.bloquear();

        // Take a ticket, like at the deli counter.
        let turno = estado.siguiente_turno;
        estado.siguiente_turno += 1;
        estado.cola.push_back(turno);

        loop {
            if cancelar.load(Ordering::Relaxed) {
                estado.cola.retain(|&t| t != turno);
                drop(estado);
                c.cambio.notify_all(); // the next one in the queue may be able to go now
                return Err(ErrorPlanificador::Cancelado);
            }
            // Only the first in the queue acts, and never while someone is loading a model.
            if estado.cola.front() == Some(&turno) && estado.cargando.is_none() {
                // a) The one we want is already loaded: lend it. (First the engine's `Arc` is copied and reading finishes; then
                //    the state is modified: Rust does not allow modifying something while it is being read.)
                let mismo = estado
                    .actual
                    .as_ref()
                    .filter(|actual| actual.carga == *carga)
                    .map(|actual| Arc::clone(&actual.motor));
                // a') …but if its process exited by itself, a dead engine is not lent: as soon as nobody uses it, it is removed
                //     and loaded again (below, like a model change).
                let mismo = match mismo {
                    Some(motor) if !c.arrancador.sigue_vivo(&motor) => {
                        if estado.prestamos == 0 {
                            soltar_actual(&mut estado, "el motor se cerró solo");
                            break;
                        }
                        None // someone holds it: their request will fail and they will release it; wait
                    }
                    otro => otro,
                };
                if let Some(motor) = mismo {
                    estado.cola.pop_front();
                    estado.prestamos += 1;
                    drop(estado);
                    c.cambio.notify_all(); // the next one may want the same model: let it through too
                    return Ok(Prestamo {
                        motor,
                        compartido: Arc::clone(c),
                    });
                }
                // b) There is another model, but nobody uses it: free to change it.
                if estado.prestamos == 0 {
                    break;
                }
                // c) There is another model in use: wait for it to be released.
            }
            estado = c.esperar_un_rato(estado);
        }

        estado.cola.pop_front();
        // Drop the old engine to stop the process (it is fast). This way the RAM is free BEFORE loading the new one.
        soltar_actual(&mut estado, "cambio de modelo");
        estado.cargando = Some(carga.modelo.clone());
        // Loading can take seconds: it is done WITHOUT the lock, so the others can query the state.
        drop(estado);

        log::info!("cargando el modelo «{}»", carga.modelo);
        let inicio = Instant::now();
        let resultado = c.arrancador.arrancar(carga);

        let mut estado = c.bloquear();
        estado.cargando = None;
        let respuesta = match resultado {
            Ok(motor) => {
                log::info!(
                    "modelo «{}» listo en {:.1} s",
                    carga.modelo,
                    inicio.elapsed().as_secs_f64()
                );
                let motor = Arc::new(motor);
                estado.actual = Some(Cargado {
                    carga: carga.clone(),
                    motor: Arc::clone(&motor),
                });
                estado.prestamos = 1;
                estado.ultimo_uso = Instant::now();
                Ok(Prestamo {
                    motor,
                    compartido: Arc::clone(c),
                })
            }
            Err(e) => {
                log::warn!("no se pudo cargar el modelo «{}»: {e}", carga.modelo);
                Err(ErrorPlanificador::Motor(e))
            }
        };
        drop(estado);
        c.cambio.notify_all(); // wake up those who were waiting
        respuesta
    }

    /// Requests waiting for their turn.
    pub fn en_cola(&self) -> usize {
        self.compartido.bloquear().cola.len()
    }

    /// What it is doing now.
    pub fn estado(&self) -> Estado {
        let estado = self.compartido.bloquear();
        if let Some(modelo) = &estado.cargando {
            return Estado::Cargando {
                modelo: modelo.clone(),
            };
        }
        match &estado.actual {
            None => Estado::Vacio,
            Some(actual) => Estado::Listo {
                modelo: actual.carga.modelo.clone(),
                en_uso: estado.prestamos,
            },
        }
    }

    /// How long until the loaded model unloads due to inactivity. `None` if there is no model, if it is being used or
    /// loaded, or if idle unloading is disabled.
    pub fn se_descarga_en(&self) -> Option<Duration> {
        let limite = self.compartido.opciones.inactividad?;
        let estado = self.compartido.bloquear();
        if estado.actual.is_none() || estado.prestamos > 0 || estado.cargando.is_some() {
            return None;
        }
        Some(limite.saturating_sub(estado.ultimo_uso.elapsed()))
    }

    /// Queries something about the loaded engine (for example, its process to measure its memory), without lending it:
    /// it does not count as use. `None` if no model is loaded.
    pub fn con_motor<T>(&self, consulta: impl FnOnce(&A::Motor) -> T) -> Option<T> {
        let estado = self.compartido.bloquear();
        estado.actual.as_ref().map(|actual| consulta(&actual.motor))
    }

    /// Unloads the model right now, if nobody is using it. Returns whether it was unloaded.
    pub fn descargar(&self) -> bool {
        let c = &self.compartido;
        let mut estado = c.bloquear();
        if estado.prestamos > 0 || estado.cargando.is_some() || estado.actual.is_none() {
            return false;
        }
        soltar_actual(&mut estado, "a petición"); // stops the engine
        c.cambio.notify_all();
        true
    }
}

impl<A: Arrancador> Drop for Planificador<A> {
    /// Shuts down the watcher. The model stops when the last loan is released (or now, if there is none).
    fn drop(&mut self) {
        {
            let mut estado = self.compartido.bloquear();
            estado.cerrando = true;
            if estado.prestamos == 0 {
                soltar_actual(&mut estado, "cierre de Locall");
            }
        }
        self.compartido.cambio.notify_all();
        if let Some(vigilante) = self.vigilante.take() {
            let _ = vigilante.join();
        }
    }
}

impl<A: Arrancador> fmt::Debug for Planificador<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Planificador")
            .field("estado", &self.estado())
            .finish_non_exhaustive()
    }
}

impl<A: Arrancador> Compartido<A> {
    /// Takes the lock. If another thread panicked while holding it, we carry on with the state as it was left: all
    /// state changes are small and always leave it consistent.
    fn bloquear(&self) -> MutexGuard<'_, Interno<A::Motor>> {
        self.estado.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Like [`Compartido::esperar`], but wakes up after at most [`MIRAR_CANCELACION`] to check whether the request
    /// has been cancelled (whoever cancels cannot notify the `Condvar`: they only change their `AtomicBool`).
    fn esperar_un_rato<'a>(
        &self,
        estado: MutexGuard<'a, Interno<A::Motor>>,
    ) -> MutexGuard<'a, Interno<A::Motor>> {
        self.cambio
            .wait_timeout(estado, MIRAR_CANCELACION)
            .unwrap_or_else(PoisonError::into_inner)
            .0
    }

    /// Releases the lock and sleeps until someone signals a change.
    fn esperar<'a>(
        &self,
        estado: MutexGuard<'a, Interno<A::Motor>>,
    ) -> MutexGuard<'a, Interno<A::Motor>> {
        self.cambio
            .wait(estado)
            .unwrap_or_else(PoisonError::into_inner)
    }
}

// ─── The loan ─────────────────────────────────────────────────────────────────

/// Permission to use the loaded model. While it exists, the model is neither changed nor unloaded.
///
/// It is used as if it were the engine (`prestamo.chat(...)`) thanks to `Deref`. Dropping it records the use.
pub struct Prestamo<A: Arrancador> {
    motor: Arc<A::Motor>,
    compartido: Arc<Compartido<A>>,
}

impl<A: Arrancador> Deref for Prestamo<A> {
    type Target = A::Motor;

    fn deref(&self) -> &A::Motor {
        &self.motor
    }
}

impl<A: Arrancador> Drop for Prestamo<A> {
    fn drop(&mut self) {
        let mut estado = self.compartido.bloquear();
        estado.prestamos -= 1;
        estado.ultimo_uso = Instant::now();
        if estado.cerrando && estado.prestamos == 0 {
            // The scheduler no longer exists: stop the engine now.
            soltar_actual(&mut estado, "cierre de Locall");
        }
        drop(estado);
        self.compartido.cambio.notify_all();
    }
}

/// Stops the loaded model (if any) and records it in the log.
fn soltar_actual<M>(estado: &mut Interno<M>, motivo: &str) {
    if let Some(viejo) = estado.actual.take() {
        let modelo = viejo.carga.modelo.clone();
        drop(viejo); // stops the engine
        log::info!("modelo «{modelo}» descargado ({motivo})");
    }
}

/// How often the watcher checks whether the engine is still alive (while a model nobody uses is loaded).
const VIGILAR_MOTOR: Duration = Duration::from_secs(2);

// ─── The inactivity watcher ───────────────────────────────────────────────────

/// Thread that unloads the model when it has gone `inactividad` without loans, and that removes the engine if its
/// process exited by itself (so the state does not say «ready» with a dead engine).
fn vigilar<A: Arrancador>(c: &Compartido<A>) {
    let mut estado = c.bloquear();
    loop {
        if estado.cerrando {
            return;
        }
        let mut espera = None; // None = sleep until the next change
        let libre = estado.actual.is_some() && estado.prestamos == 0 && estado.cargando.is_none();
        if libre {
            let muerto = estado
                .actual
                .as_ref()
                .is_some_and(|actual| !c.arrancador.sigue_vivo(&actual.motor));
            if muerto {
                soltar_actual(&mut estado, "el motor se cerró solo");
                c.cambio.notify_all();
                continue;
            }
            espera = Some(VIGILAR_MOTOR);
        }
        if let Some(limite) = c.opciones.inactividad
            && libre
        {
            let pasado = estado.ultimo_uso.elapsed();
            if pasado >= limite {
                soltar_actual(&mut estado, "inactividad"); // stops the engine
                c.cambio.notify_all();
                continue;
            }
            espera = Some((limite - pasado).min(VIGILAR_MOTOR));
        }
        estado = match espera {
            Some(d) => {
                c.cambio
                    .wait_timeout(estado, d)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0
            }
            None => c.esperar(estado),
        };
    }
}
