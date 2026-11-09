//! # locall-scheduler
//!
//! El planificador decide **qué modelo está cargado**. La app, la API y la CLI no hablan con el motor
//! directamente: piden al planificador "quiero usar este modelo" y reciben un [`Prestamo`].
//!
//! Reglas:
//!
//! 1. **Un solo modelo a la vez** (cabe en equipos de 16 GB; decisión de la Fase 0). Si piden el mismo modelo con la
//!    misma configuración, se reutiliza; si piden otro, se para el actual y se arranca el nuevo.
//! 2. Mientras alguien tiene un **préstamo**, el modelo no se cambia: quien pide otro modelo **espera**.
//!    Varios pueden tener a la vez un préstamo del *mismo* modelo.
//! 3. Si el modelo lleva [`Opciones::inactividad`] sin usarse y nadie lo tiene prestado, **se descarga solo** para
//!    liberar la RAM.
//! 4. **Cola por orden de llegada:** se atiende a quien llegó primero. Quien pide el modelo ya cargado no se cuela
//!    por delante de quien espera otro modelo (si no, ese podría esperar para siempre). Se puede **cancelar**
//!    mientras se espera turno.
//!
//! El planificador no conoce `llama-server`: trabaja con cualquier [`Arrancador`]. Locall usa [`ArrancadorReal`]
//! (el motor de `locall-engine`); las pruebas usan uno de imitación, rápido y exacto.

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

// ─── El trait Arrancador ──────────────────────────────────────────────────────

/// Lo que el planificador necesita de un motor: saber **arrancarlo** con una carga.
///
/// Para **pararlo** basta con soltarlo (`Drop`), como ya hace [`locall_engine::Motor`].
///
/// `Send + Sync + 'static` significa: se puede compartir entre hilos y no toma prestado nada de fuera. El
/// planificador lo necesita porque lo usa desde varios hilos y desde su vigilante.
pub trait Arrancador: Send + Sync + 'static {
    /// El motor en marcha que devuelve.
    type Motor: Send + Sync + 'static;
    /// El error si no puede arrancar.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Arranca un motor con `carga` y espera a que esté listo.
    fn arrancar(&self, carga: &Carga) -> Result<Self::Motor, Self::Error>;
}

/// El arrancador de verdad: `llama-server` mediante `locall-engine`.
///
/// Antes de lanzar el motor comprueba (parte 9c) que el modelo es compatible y que **cabe en la memoria
/// disponible**; si no, devuelve el error al instante, sin arrancar nada.
#[derive(Debug, Clone)]
pub struct ArrancadorReal {
    /// Tiempo máximo para cargar un modelo.
    pub espera_maxima: Duration,
    /// Comprobar compatibilidad y memoria antes de cargar. Solo se desactiva si el usuario insiste en probar
    /// (la estimación es prudente y puede equivocarse por poco).
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
            // El planificador ya ha parado el modelo anterior: la memoria que libera cuenta como disponible.
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
}

// ─── Tipos públicos ───────────────────────────────────────────────────────────

/// Qué modelo cargar y cómo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Carga {
    /// Nombre del modelo para mostrarlo (por ejemplo, `qwen2.5-1.5b`). Nunca una ruta.
    pub modelo: String,
    /// Cómo arrancar el motor. Si cambia algo (el contexto, las capas en la gráfica…), es otra carga.
    pub config: ConfigMotor,
}

/// Ajustes del planificador.
#[derive(Debug, Clone)]
pub struct Opciones {
    /// Tiempo sin usarse tras el que el modelo se descarga solo. `None` = no se descarga nunca solo.
    pub inactividad: Option<Duration>,
}

impl Default for Opciones {
    fn default() -> Self {
        Opciones {
            inactividad: Some(Duration::from_secs(5 * 60)),
        }
    }
}

/// Qué está haciendo el planificador.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Estado {
    /// No hay ningún modelo cargado.
    Vacio,
    /// Se está cargando este modelo.
    Cargando { modelo: String },
    /// Este modelo está listo. `en_uso` = préstamos activos.
    Listo { modelo: String, en_uso: usize },
}

/// Lo que puede salir mal al pedir un modelo.
#[derive(Debug)]
pub enum ErrorPlanificador<E> {
    /// Se canceló mientras esperaba su turno.
    Cancelado,
    /// El motor no pudo arrancar.
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

/// Cada cuánto mira la petición en espera si la han cancelado.
const MIRAR_CANCELACION: Duration = Duration::from_millis(50);

// ─── El planificador ─────────────────────────────────────────────────────────

/// El planificador. Se comparte entre hilos con una referencia (`&Planificador`) o un `Arc<Planificador>`.
pub struct Planificador<A: Arrancador> {
    compartido: Arc<Compartido<A>>,
    vigilante: Option<JoinHandle<()>>,
}

/// Lo que comparten el planificador, los préstamos y el vigilante.
struct Compartido<A: Arrancador> {
    arrancador: A,
    opciones: Opciones,
    estado: Mutex<Interno<A::Motor>>,
    /// Avisa a los que esperan cada vez que cambia el estado.
    cambio: Condvar,
}

/// El estado, protegido por el `Mutex`: solo un hilo a la vez puede leerlo o cambiarlo.
struct Interno<M> {
    actual: Option<Cargado<M>>,
    /// Modelo que algún hilo está cargando ahora mismo (los demás esperan).
    cargando: Option<String>,
    /// Préstamos activos del modelo actual.
    prestamos: usize,
    /// Última vez que se soltó un préstamo (o que se cargó el modelo).
    ultimo_uso: Instant,
    /// El planificador se está cerrando: el vigilante debe terminar.
    cerrando: bool,
    /// Turnos de las peticiones que esperan, por orden de llegada.
    cola: VecDeque<u64>,
    /// Número del próximo turno que se reparte.
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

    /// Pide usar el modelo de `carga`. Si hace falta, para el modelo actual y carga este.
    ///
    /// **Bloquea** hasta que sea su turno. Para poder cancelar la espera, usa [`Planificador::usar_con`].
    pub fn usar(&self, carga: &Carga) -> Result<Prestamo<A>, ErrorPlanificador<A::Error>> {
        self.usar_con(carga, &AtomicBool::new(false))
    }

    /// Como [`Planificador::usar`], pero si otro hilo pone `cancelar` a `true` mientras espera su turno, se va de
    /// la cola con [`ErrorPlanificador::Cancelado`]. (Una carga ya empezada no se interrumpe.)
    pub fn usar_con(
        &self,
        carga: &Carga,
        cancelar: &AtomicBool,
    ) -> Result<Prestamo<A>, ErrorPlanificador<A::Error>> {
        let c = &self.compartido;
        let mut estado = c.bloquear();

        // Coger turno, como en la pescadería.
        let turno = estado.siguiente_turno;
        estado.siguiente_turno += 1;
        estado.cola.push_back(turno);

        loop {
            if cancelar.load(Ordering::Relaxed) {
                estado.cola.retain(|&t| t != turno);
                drop(estado);
                c.cambio.notify_all(); // el siguiente de la cola quizá ya puede pasar
                return Err(ErrorPlanificador::Cancelado);
            }
            // Solo actúa el primero de la cola, y nunca mientras alguien está cargando un modelo.
            if estado.cola.front() == Some(&turno) && estado.cargando.is_none() {
                // a) Ya está cargado el que queremos: prestarlo. (Primero se copia el `Arc` del motor y se
                //    termina de leer; después se modifica el estado: Rust no deja modificar algo mientras se lee.)
                let mismo = estado
                    .actual
                    .as_ref()
                    .filter(|actual| actual.carga == *carga)
                    .map(|actual| Arc::clone(&actual.motor));
                if let Some(motor) = mismo {
                    estado.cola.pop_front();
                    estado.prestamos += 1;
                    drop(estado);
                    c.cambio.notify_all(); // el siguiente quizá quiere el mismo modelo: que pase también
                    return Ok(Prestamo {
                        motor,
                        compartido: Arc::clone(c),
                    });
                }
                // b) Hay otro modelo, pero nadie lo usa: vía libre para cambiarlo.
                if estado.prestamos == 0 {
                    break;
                }
                // c) Hay otro modelo en uso: esperar a que lo suelten.
            }
            estado = c.esperar_un_rato(estado);
        }

        estado.cola.pop_front();
        // Soltar el motor viejo para el proceso (es rápido). Así la RAM queda libre ANTES de cargar el nuevo.
        soltar_actual(&mut estado, "cambio de modelo");
        estado.cargando = Some(carga.modelo.clone());
        // Cargar puede tardar segundos: se hace SIN el candado, para que los demás puedan consultar el estado.
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
        c.cambio.notify_all(); // despertar a los que esperaban
        respuesta
    }

    /// Peticiones esperando turno.
    pub fn en_cola(&self) -> usize {
        self.compartido.bloquear().cola.len()
    }

    /// Qué está haciendo ahora.
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

    /// Cuánto falta para que el modelo cargado se descargue por inactividad. `None` si no hay modelo, si se está
    /// usando o cargando, o si la descarga por inactividad está desactivada.
    pub fn se_descarga_en(&self) -> Option<Duration> {
        let limite = self.compartido.opciones.inactividad?;
        let estado = self.compartido.bloquear();
        if estado.actual.is_none() || estado.prestamos > 0 || estado.cargando.is_some() {
            return None;
        }
        Some(limite.saturating_sub(estado.ultimo_uso.elapsed()))
    }

    /// Consulta algo del motor cargado (por ejemplo, su proceso para medir su memoria), sin prestarlo: no cuenta como
    /// uso. `None` si no hay modelo cargado.
    pub fn con_motor<T>(&self, consulta: impl FnOnce(&A::Motor) -> T) -> Option<T> {
        let estado = self.compartido.bloquear();
        estado.actual.as_ref().map(|actual| consulta(&actual.motor))
    }

    /// Descarga el modelo ahora mismo, si nadie lo está usando. Devuelve si se ha descargado.
    pub fn descargar(&self) -> bool {
        let c = &self.compartido;
        let mut estado = c.bloquear();
        if estado.prestamos > 0 || estado.cargando.is_some() || estado.actual.is_none() {
            return false;
        }
        soltar_actual(&mut estado, "a petición"); // para el motor
        c.cambio.notify_all();
        true
    }
}

impl<A: Arrancador> Drop for Planificador<A> {
    /// Cierra el vigilante. El modelo se para cuando se suelte el último préstamo (o ahora, si no hay ninguno).
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
    /// Toma el candado. Si otro hilo entró en pánico con él tomado, seguimos con el estado tal como quedó: todos
    /// los cambios del estado son pequeños y quedan siempre coherentes.
    fn bloquear(&self) -> MutexGuard<'_, Interno<A::Motor>> {
        self.estado.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Como [`Compartido::esperar`], pero se despierta como mucho a los [`MIRAR_CANCELACION`] para comprobar si
    /// han cancelado la petición (quien cancela no puede avisar a la `Condvar`: solo cambia su `AtomicBool`).
    fn esperar_un_rato<'a>(
        &self,
        estado: MutexGuard<'a, Interno<A::Motor>>,
    ) -> MutexGuard<'a, Interno<A::Motor>> {
        self.cambio
            .wait_timeout(estado, MIRAR_CANCELACION)
            .unwrap_or_else(PoisonError::into_inner)
            .0
    }

    /// Suelta el candado y duerme hasta que alguien avise de un cambio.
    fn esperar<'a>(
        &self,
        estado: MutexGuard<'a, Interno<A::Motor>>,
    ) -> MutexGuard<'a, Interno<A::Motor>> {
        self.cambio
            .wait(estado)
            .unwrap_or_else(PoisonError::into_inner)
    }
}

// ─── El préstamo ─────────────────────────────────────────────────────────────

/// Permiso para usar el modelo cargado. Mientras exista, el modelo no se cambia ni se descarga.
///
/// Se usa como si fuera el motor (`prestamo.chat(...)`) gracias a `Deref`. Al soltarlo, se anota el uso.
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
            // El planificador ya no existe: parar el motor ahora.
            soltar_actual(&mut estado, "cierre de Locall");
        }
        drop(estado);
        self.compartido.cambio.notify_all();
    }
}

/// Para el modelo cargado (si lo hay) y lo anota en el registro.
fn soltar_actual<M>(estado: &mut Interno<M>, motivo: &str) {
    if let Some(viejo) = estado.actual.take() {
        let modelo = viejo.carga.modelo.clone();
        drop(viejo); // para el motor
        log::info!("modelo «{modelo}» descargado ({motivo})");
    }
}

// ─── El vigilante de inactividad ─────────────────────────────────────────────

/// Hilo que descarga el modelo cuando lleva `inactividad` sin préstamos.
fn vigilar<A: Arrancador>(c: &Compartido<A>) {
    let mut estado = c.bloquear();
    loop {
        if estado.cerrando {
            return;
        }
        let mut espera = None; // None = dormir hasta el próximo cambio
        if let Some(limite) = c.opciones.inactividad
            && estado.actual.is_some()
            && estado.prestamos == 0
            && estado.cargando.is_none()
        {
            let pasado = estado.ultimo_uso.elapsed();
            if pasado >= limite {
                soltar_actual(&mut estado, "inactividad"); // para el motor
                c.cambio.notify_all();
                continue;
            }
            espera = Some(limite - pasado);
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
