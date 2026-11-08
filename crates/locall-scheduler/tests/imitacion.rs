//! Pruebas del planificador con un **motor de imitación**: "carga" al instante (o con la demora que digamos) y
//! apunta todo lo que le piden. Así las pruebas con varios hilos son rápidas y exactas.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use locall_engine::ConfigMotor;
use locall_scheduler::{Arrancador, Carga, Estado, Opciones, Planificador};

// ─── El motor de imitación ───────────────────────────────────────────────────

/// Lleva la cuenta de lo que pasa, compartida con la prueba.
#[derive(Default)]
struct Registro {
    /// Modelos cargados, en orden.
    cargas: Mutex<Vec<String>>,
    /// Motores "en marcha" ahora mismo.
    vivos: AtomicUsize,
    /// Máximo de motores en marcha a la vez que se ha visto (debe ser 1).
    max_vivos: AtomicUsize,
}

struct Imitacion {
    registro: Arc<Registro>,
    demora: Duration,
    /// Modelos que fallan al cargar.
    fallan: Vec<String>,
}

struct MotorFalso {
    modelo: String,
    registro: Arc<Registro>,
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
        })
    }
}

// ─── Ayudantes ────────────────────────────────────────────────────────────────

fn carga(modelo: &str) -> Carga {
    Carga {
        modelo: modelo.into(),
        config: ConfigMotor {
            ejecutable: PathBuf::from("llama-server.exe"),
            modelo: PathBuf::from(format!("{modelo}.gguf")),
            contexto: 4096,
            hilos: 1,
            capas_gpu: 0,
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

// ─── Pruebas ──────────────────────────────────────────────────────────────────

#[test]
fn reutiliza_el_mismo_modelo() {
    let (plan, reg) = planificador(0, None);
    drop(plan.usar(&carga("a")).unwrap());
    let prestamo = plan.usar(&carga("a")).unwrap();
    assert_eq!(prestamo.modelo, "a"); // gracias a Deref, el préstamo se usa como el motor
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
    let (plan, reg) = planificador(100, None); // la carga tarda 100 ms
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

    // El plazo cuenta desde que se suelta.
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
    // Quien estaba usando el modelo puede terminar su respuesta...
    assert_eq!(prestamo.modelo, "a");
    assert_eq!(reg.vivos.load(Ordering::SeqCst), 1);
    // ...y al soltarlo, el motor se para.
    drop(prestamo);
    assert_eq!(reg.vivos.load(Ordering::SeqCst), 0);
}
