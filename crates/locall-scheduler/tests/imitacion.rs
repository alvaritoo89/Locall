//! Scheduler tests with a **fake engine**: it "loads" instantly (or with the delay we say) and records everything
//! it is asked. This way multi-threaded tests are fast and exact.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use locall_engine::ConfigMotor;
use locall_scheduler::{Arrancador, Carga, ErrorPlanificador, Estado, Opciones, Planificador};

// ─── The fake engine ──────────────────────────────────────────────────────────

/// Keeps track of what happens, shared with the test.
#[derive(Default)]
struct Registro {
    /// Models loaded, in order.
    cargas: Mutex<Vec<String>>,
    /// Engines "running" right now.
    vivos: AtomicUsize,
    /// Maximum number of engines seen running at once (must be 1).
    max_vivos: AtomicUsize,
}

struct Imitacion {
    registro: Arc<Registro>,
    demora: Duration,
    /// Models that fail to load.
    fallan: Vec<String>,
}

struct MotorFalso {
    modelo: String,
    registro: Arc<Registro>,
    /// The test sets it to `true` to fake an engine whose process exited by itself.
    muerto: AtomicBool,
}

impl Drop for MotorFalso {
    fn drop(&mut self) {
        self.registro.vivos.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Debug)]
struct FalloFalso;

impl std::fmt::Display for FalloFalso {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "fallo de imitación")
    }
}

impl std::error::Error for FalloFalso {}

impl Arrancador for Imitacion {
    type Motor = MotorFalso;
    type Error = FalloFalso;

    fn arrancar(&self, carga: &Carga) -> Result<MotorFalso, FalloFalso> {
        thread::sleep(self.demora);
        if self.fallan.contains(&carga.modelo) {
            return Err(FalloFalso);
        }
        self.registro
            .cargas
            .lock()
            .unwrap()
            .push(carga.modelo.clone());
        let vivos = self.registro.vivos.fetch_add(1, Ordering::SeqCst) + 1;
        self.registro.max_vivos.fetch_max(vivos, Ordering::SeqCst);
        Ok(MotorFalso {
            modelo: carga.modelo.clone(),
            registro: Arc::clone(&self.registro),
            muerto: AtomicBool::new(false),
        })
    }

    fn sigue_vivo(&self, motor: &MotorFalso) -> bool {
        !motor.muerto.load(Ordering::SeqCst)
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn carga(modelo: &str) -> Carga {
    Carga {
        modelo: modelo.into(),
        config: ConfigMotor {
            ejecutable: PathBuf::from("llama-server.exe"),
            modelo: PathBuf::from(format!("{modelo}.gguf")),
            contexto: 4096,
            hilos: 1,
            capas_gpu: 0,
            paralelo: 1,
            registro: PathBuf::from("motor.log"),
        },
    }
}

fn planificador(
    demora_ms: u64,
    inactividad: Option<Duration>,
) -> (Planificador<Imitacion>, Arc<Registro>) {
    planificador_con_fallos(demora_ms, inactividad, &[])
}

fn planificador_con_fallos(
    demora_ms: u64,
    inactividad: Option<Duration>,
    fallan: &[&str],
) -> (Planificador<Imitacion>, Arc<Registro>) {
    let registro = Arc::new(Registro::default());
    let arrancador = Imitacion {
        registro: Arc::clone(&registro),
        demora: Duration::from_millis(demora_ms),
        fallan: fallan.iter().map(|s| s.to_string()).collect(),
    };
    (
        Planificador::nuevo(arrancador, Opciones { inactividad }),
        registro,
    )
}

fn cargas(registro: &Registro) -> Vec<String> {
    registro.cargas.lock().unwrap().clone()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[test]
fn reutiliza_el_mismo_modelo() {
    let (plan, reg) = planificador(0, None);
    drop(plan.usar(&carga("a")).unwrap());
    let prestamo = plan.usar(&carga("a")).unwrap();
    assert_eq!(prestamo.modelo, "a"); // thanks to Deref, the loan is used like the engine
    assert_eq!(cargas(&reg), ["a"], "no debe cargarlo dos veces");
}

#[test]
fn otra_configuracion_es_otra_carga() {
    let (plan, reg) = planificador(0, None);
    drop(plan.usar(&carga("a")).unwrap());
    let mut mas_contexto = carga("a");
    mas_contexto.config.contexto = 8192;
    drop(plan.usar(&mas_contexto).unwrap());
    assert_eq!(cargas(&reg), ["a", "a"]);
    assert_eq!(reg.vivos.load(Ordering::SeqCst), 1);
}

#[test]
fn cambia_de_modelo_parando_el_anterior() {
    let (plan, reg) = planificador(0, None);
    drop(plan.usar(&carga("a")).unwrap());
    drop(plan.usar(&carga("b")).unwrap());
    assert_eq!(cargas(&reg), ["a", "b"]);
    assert_eq!(
        reg.vivos.load(Ordering::SeqCst),
        1,
        "solo queda b en marcha"
    );
    assert_eq!(
        reg.max_vivos.load(Ordering::SeqCst),
        1,
        "nunca dos a la vez"
    );
}

#[test]
fn varios_comparten_el_mismo_modelo() {
    let (plan, _reg) = planificador(0, None);
    let p1 = plan.usar(&carga("a")).unwrap();
    let p2 = plan.usar(&carga("a")).unwrap();
    assert_eq!(
        plan.estado(),
        Estado::Listo {
            modelo: "a".into(),
            en_uso: 2
        }
    );
    drop(p1);
    drop(p2);
    assert_eq!(
        plan.estado(),
        Estado::Listo {
            modelo: "a".into(),
            en_uso: 0
        }
    );
}

#[test]
fn quien_pide_otro_modelo_espera_a_que_suelten_el_actual() {
    let (plan, reg) = planificador(0, None);
    let soltado = Mutex::new(None::<Instant>);

    thread::scope(|s| {
        let prestamo_a = plan.usar(&carga("a")).unwrap();
        let espera = s.spawn(|| {
            let _b = plan.usar(&carga("b")).unwrap();
            Instant::now()
        });
        thread::sleep(Duration::from_millis(200));
        assert_eq!(
            cargas(&reg),
            ["a"],
            "b no debe cargarse mientras a está en uso"
        );
        *soltado.lock().unwrap() = Some(Instant::now());
        drop(prestamo_a);

        let cargado_b = espera.join().unwrap();
        assert!(cargado_b >= soltado.lock().unwrap().unwrap());
    });
    assert_eq!(cargas(&reg), ["a", "b"]);
    assert_eq!(reg.max_vivos.load(Ordering::SeqCst), 1);
}

#[test]
fn si_varios_piden_a_la_vez_se_carga_una_sola_vez() {
    let (plan, reg) = planificador(100, None); // loading takes 100 ms
    thread::scope(|s| {
        for _ in 0..8 {
            s.spawn(|| {
                let _p = plan.usar(&carga("a")).unwrap();
                thread::sleep(Duration::from_millis(20));
            });
        }
    });
    assert_eq!(cargas(&reg), ["a"]);
}

#[test]
fn muestra_que_esta_cargando() {
    let (plan, _reg) = planificador(300, None);
    thread::scope(|s| {
        s.spawn(|| drop(plan.usar(&carga("a")).unwrap()));
        thread::sleep(Duration::from_millis(100));
        assert_eq!(plan.estado(), Estado::Cargando { modelo: "a".into() });
    });
    assert!(matches!(plan.estado(), Estado::Listo { .. }));
}

#[test]
fn se_descarga_solo_tras_un_rato_sin_uso() {
    let (plan, reg) = planificador(0, Some(Duration::from_millis(200)));
    drop(plan.usar(&carga("a")).unwrap());
    assert_eq!(reg.vivos.load(Ordering::SeqCst), 1);

    thread::sleep(Duration::from_millis(500));
    assert_eq!(reg.vivos.load(Ordering::SeqCst), 0);
    assert_eq!(plan.estado(), Estado::Vacio);
}

#[test]
fn no_se_descarga_mientras_alguien_lo_usa() {
    let (plan, reg) = planificador(0, Some(Duration::from_millis(100)));
    let prestamo = plan.usar(&carga("a")).unwrap();
    thread::sleep(Duration::from_millis(400));
    assert_eq!(
        reg.vivos.load(Ordering::SeqCst),
        1,
        "en uso: no se descarga"
    );

    // The deadline counts from when it is released.
    drop(prestamo);
    thread::sleep(Duration::from_millis(50));
    assert_eq!(reg.vivos.load(Ordering::SeqCst), 1);
    thread::sleep(Duration::from_millis(300));
    assert_eq!(reg.vivos.load(Ordering::SeqCst), 0);
}

#[test]
fn descargar_a_mano() {
    let (plan, reg) = planificador(0, None);
    let prestamo = plan.usar(&carga("a")).unwrap();
    assert!(!plan.descargar(), "no se descarga si está en uso");
    drop(prestamo);
    assert!(plan.descargar());
    assert_eq!(reg.vivos.load(Ordering::SeqCst), 0);
    assert!(!plan.descargar(), "ya no hay nada que descargar");
}

#[test]
fn un_fallo_de_carga_no_bloquea_al_siguiente() {
    let (plan, reg) = planificador_con_fallos(0, None, &["roto"]);
    assert!(plan.usar(&carga("roto")).is_err());
    assert_eq!(plan.estado(), Estado::Vacio);
    let prestamo = plan.usar(&carga("b")).unwrap();
    assert_eq!(prestamo.modelo, "b");
    assert_eq!(cargas(&reg), ["b"]);
}

#[test]
fn al_soltar_el_planificador_se_para_el_motor() {
    let (plan, reg) = planificador(0, None);
    drop(plan.usar(&carga("a")).unwrap());
    drop(plan);
    assert_eq!(reg.vivos.load(Ordering::SeqCst), 0);
}

#[test]
fn un_prestamo_sobrevive_al_planificador() {
    let (plan, reg) = planificador(0, None);
    let prestamo = plan.usar(&carga("a")).unwrap();
    drop(plan);
    // Whoever was using the model can finish their answer...
    assert_eq!(prestamo.modelo, "a");
    assert_eq!(reg.vivos.load(Ordering::SeqCst), 1);
    // ...and when they release it, the engine stops.
    drop(prestamo);
    assert_eq!(reg.vivos.load(Ordering::SeqCst), 0);
}

// ─── Queue (part 9b) ──────────────────────────────────────────────────────────

/// Waits (up to 2 s) for there to be `n` requests in the queue.
fn espera_cola(plan: &Planificador<Imitacion>, n: usize) {
    let inicio = Instant::now();
    while plan.en_cola() != n {
        assert!(
            inicio.elapsed() < Duration::from_secs(2),
            "la cola no llegó a {n}"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn atiende_por_orden_de_llegada() {
    let (plan, reg) = planificador(0, None);
    thread::scope(|s| {
        let prestamo_a = plan.usar(&carga("a")).unwrap();
        // b, c and d arrive in this order while a is in use.
        for (i, modelo) in ["b", "c", "d"].into_iter().enumerate() {
            let plan = &plan;
            s.spawn(move || drop(plan.usar(&carga(modelo)).unwrap()));
            espera_cola(plan, i + 1);
        }
        drop(prestamo_a);
    });
    assert_eq!(cargas(&reg), ["a", "b", "c", "d"]);
}

#[test]
fn nadie_se_cuela_por_delante_de_quien_espera_otro_modelo() {
    let (plan, reg) = planificador(0, None);
    thread::scope(|s| {
        let prestamo_a = plan.usar(&carga("a")).unwrap();
        // First comes whoever wants b (they have to wait for a to be released)...
        let b = s.spawn(|| {
            let p = plan.usar(&carga("b")).unwrap();
            thread::sleep(Duration::from_millis(100));
            drop(p);
        });
        espera_cola(&plan, 1);
        // ...and then whoever wants a, which IS already loaded. It must not jump the queue: it waits behind b.
        let a_otra_vez = s.spawn(|| drop(plan.usar(&carga("a")).unwrap()));
        espera_cola(&plan, 2);
        thread::sleep(Duration::from_millis(100));
        assert_eq!(plan.en_cola(), 2, "quien pidió a no se ha colado");

        drop(prestamo_a);
        b.join().unwrap();
        a_otra_vez.join().unwrap();
    });
    // a → b → a: the second "a" was served after b.
    assert_eq!(cargas(&reg), ["a", "b", "a"]);
}

#[test]
fn los_que_quieren_el_mismo_modelo_pasan_juntos() {
    let (plan, reg) = planificador(0, None);
    thread::scope(|s| {
        let prestamo_a = plan.usar(&carga("a")).unwrap();
        // Three consecutive requests wanting b wait behind a.
        for n in 1..=3 {
            s.spawn(|| {
                let p = plan.usar(&carga("b")).unwrap();
                thread::sleep(Duration::from_millis(300)); // they use it for a while
                drop(p);
            });
            espera_cola(&plan, n);
        }
        drop(prestamo_a);

        // When a is released: b loads once and all three use it at the same time.
        let inicio = Instant::now();
        while plan.estado()
            != (Estado::Listo {
                modelo: "b".into(),
                en_uso: 3,
            })
        {
            assert!(
                inicio.elapsed() < Duration::from_millis(250),
                "los tres deberían estar usando b a la vez: {:?}",
                plan.estado()
            );
            thread::sleep(Duration::from_millis(5));
        }
    });
    assert_eq!(cargas(&reg), ["a", "b"]);
}

#[test]
fn se_puede_cancelar_mientras_se_espera_turno() {
    let (plan, reg) = planificador(0, None);
    let cancelar = AtomicBool::new(false);
    thread::scope(|s| {
        let _prestamo_a = plan.usar(&carga("a")).unwrap();
        let espera = s.spawn(|| plan.usar_con(&carga("b"), &cancelar));
        espera_cola(&plan, 1);

        let inicio = Instant::now();
        cancelar.store(true, Ordering::Relaxed);
        let resultado = espera.join().unwrap();

        assert!(matches!(resultado, Err(ErrorPlanificador::Cancelado)));
        assert!(
            inicio.elapsed() < Duration::from_millis(500),
            "tardó en enterarse"
        );
        assert_eq!(plan.en_cola(), 0, "se ha ido de la cola");
    });
    assert_eq!(cargas(&reg), ["a"], "b no llegó a cargarse");
}

#[test]
fn cancelar_al_primero_deja_pasar_al_siguiente() {
    let (plan, reg) = planificador(0, None);
    let cancelar_b = AtomicBool::new(false);
    thread::scope(|s| {
        let prestamo_a = plan.usar(&carga("a")).unwrap();
        let b = s.spawn(|| plan.usar_con(&carga("b"), &cancelar_b).map(drop));
        espera_cola(&plan, 1);
        let c = s.spawn(|| plan.usar(&carga("c")).map(drop));
        espera_cola(&plan, 2);

        cancelar_b.store(true, Ordering::Relaxed);
        assert!(matches!(
            b.join().unwrap(),
            Err(ErrorPlanificador::Cancelado)
        ));
        drop(prestamo_a);
        c.join().unwrap().unwrap();
    });
    assert_eq!(cargas(&reg), ["a", "c"]);
}

// ─── Queries for the app (part 5b) ────────────────────────────────────────────

#[test]
fn dice_cuanto_falta_para_descargarse() {
    let (plan, _reg) = planificador(0, Some(Duration::from_secs(60)));
    assert_eq!(
        plan.se_descarga_en(),
        None,
        "sin modelo, nada que descargar"
    );
    let prestamo = plan.usar(&carga("a")).unwrap();
    assert_eq!(plan.se_descarga_en(), None, "en uso, no cuenta");
    drop(prestamo);
    let falta = plan.se_descarga_en().unwrap();
    assert!(
        falta <= Duration::from_secs(60) && falta > Duration::from_secs(58),
        "{falta:?}"
    );

    let (sin_limite, _) = planificador(0, None);
    drop(sin_limite.usar(&carga("a")).unwrap());
    assert_eq!(
        sin_limite.se_descarga_en(),
        None,
        "sin descarga por inactividad"
    );
}

#[test]
fn consulta_el_motor_sin_contarlo_como_uso() {
    let (plan, _reg) = planificador(0, None);
    assert_eq!(plan.con_motor(|m| m.modelo.clone()), None);
    drop(plan.usar(&carga("a")).unwrap());
    assert_eq!(plan.con_motor(|m| m.modelo.clone()).as_deref(), Some("a"));
    assert_eq!(
        plan.estado(),
        Estado::Listo {
            modelo: "a".into(),
            en_uso: 0
        },
        "consultar no es prestar"
    );
}

// ─── Engine that exits by itself (Phase 7a) ───────────────────────────────────

#[test]
fn si_el_motor_se_cierra_solo_se_vuelve_a_cargar() {
    let (plan, reg) = planificador(0, None);
    let prestamo = plan.usar(&carga("a")).unwrap();
    prestamo.muerto.store(true, Ordering::SeqCst); // the engine process exits (out of memory…)
    drop(prestamo);

    // When the same model is requested, the dead engine is not lent: it is removed and another one is loaded.
    let otro = plan.usar(&carga("a")).unwrap();
    assert!(
        !otro.muerto.load(Ordering::SeqCst),
        "el motor prestado está vivo"
    );
    assert_eq!(cargas(&reg), ["a", "a"]);
    assert_eq!(reg.vivos.load(Ordering::SeqCst), 1, "el muerto se soltó");
}

#[test]
fn el_vigilante_retira_un_motor_que_se_cerro_solo() {
    // No idle unloading: even so, a dead engine must not stay «ready».
    let (plan, reg) = planificador(0, None);
    let prestamo = plan.usar(&carga("a")).unwrap();
    prestamo.muerto.store(true, Ordering::SeqCst);
    drop(prestamo);
    assert!(matches!(plan.estado(), Estado::Listo { .. }));

    let limite = Instant::now() + Duration::from_secs(5);
    while plan.estado() != Estado::Vacio && Instant::now() < limite {
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(plan.estado(), Estado::Vacio, "el vigilante lo retiró");
    assert_eq!(reg.vivos.load(Ordering::SeqCst), 0);
}

#[test]
fn un_motor_muerto_no_se_retira_mientras_alguien_lo_tiene_prestado() {
    let (plan, reg) = planificador(0, None);
    let plan = Arc::new(plan);
    let primero = plan.usar(&carga("a")).unwrap();
    primero.muerto.store(true, Ordering::SeqCst);

    // Someone else requests the same model: the dead one is not lent to them, nor is it taken from the first one (their
    // request will fail on its own): they wait until it is released and then a new one is loaded.
    let otro_plan = Arc::clone(&plan);
    let segundo = thread::spawn(move || {
        let p = otro_plan.usar(&carga("a")).unwrap();
        !p.muerto.load(Ordering::SeqCst)
    });
    thread::sleep(Duration::from_millis(200));
    assert_eq!(cargas(&reg), ["a"], "todavía no ha recargado");
    drop(primero);
    assert!(segundo.join().unwrap(), "al segundo le tocó un motor vivo");
    assert_eq!(cargas(&reg), ["a", "a"]);
}
