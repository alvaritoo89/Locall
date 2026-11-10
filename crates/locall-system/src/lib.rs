//! # locall-system
//!
//! Lo que Locall necesita saber del equipo y del sistema operativo:
//!
//! - [`memoria`]: RAM total y disponible **de verdad** (la regla de memoria no se fía de lo que dice Vulkan en las
//!   gráficas integradas, ADR 0007).
//! - [`procesador`] e [`hilos_para_motor`]: nombre del procesador e hilos a usar (decisión D5), con validación
//!   ([`validar_hilos`]) para que nunca se repita el error de los 246 hilos de la Fase 0.
//! - [`memoria_de_proceso`]: cuánta RAM usa un proceso (por ejemplo, el motor), medido por el sistema.
//! - [`Trabajo`]: el Job Object de Windows que garantiza que el motor muere si Locall muere (ADR 0003).
//!
//! Toda la comunicación con Windows (`unsafe`) vive en el módulo privado `windows`.

use std::fmt;
use std::io;
use std::process::Child;

#[cfg(windows)]
mod windows;

// ─── Memoria del equipo ───────────────────────────────────────────────────────

/// La memoria RAM del equipo, en bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Memoria {
    pub total: u64,
    /// Lo que se puede usar ahora mismo sin que Windows tenga que mover cosas al disco.
    pub disponible: u64,
}

/// Consulta la RAM total y disponible.
pub fn memoria() -> io::Result<Memoria> {
    #[cfg(windows)]
    {
        windows::memoria()
    }
    #[cfg(not(windows))]
    {
        Err(no_soportado())
    }
}

// ─── Procesador e hilos ───────────────────────────────────────────────────────

/// El procesador del equipo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Procesador {
    /// Nombre comercial, por ejemplo `13th Gen Intel(R) Core(TM) i5-1334U`, si se puede averiguar.
    pub nombre: Option<String>,
    /// Hilos que puede ejecutar a la vez.
    pub hilos_logicos: usize,
}

/// Información del procesador.
pub fn procesador() -> Procesador {
    #[cfg(windows)]
    let nombre = windows::nombre_procesador();
    #[cfg(not(windows))]
    let nombre = None;
    Procesador {
        nombre,
        hilos_logicos: hilos_logicos(),
    }
}

/// Hilos lógicos del equipo (al menos 1).
pub fn hilos_logicos() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

/// Hilos que Locall da al motor por defecto: todos los lógicos (decisión D5 de la Fase 0: la lectura del prompt
/// escala hasta 12 hilos y la escritura no empeora).
pub fn hilos_para_motor() -> usize {
    hilos_logicos()
}

/// Error al pedir un número de hilos imposible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HilosInvalidos {
    pub pedidos: usize,
    pub maximo: usize,
}

impl fmt::Display for HilosInvalidos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} hilos no es válido: debe estar entre 1 y {} (los hilos de este equipo)",
            self.pedidos, self.maximo
        )
    }
}

impl std::error::Error for HilosInvalidos {}

/// Comprueba que un número de hilos pedido (por el usuario o la configuración) es posible en este equipo.
///
/// En la Fase 0, un fallo hizo que el motor arrancara con 246 hilos en un equipo de 12: la escritura cayó de
/// ~33 a 2,1 tokens/s. Esta validación lo impide.
pub fn validar_hilos(pedidos: usize) -> Result<usize, HilosInvalidos> {
    let maximo = hilos_logicos();
    if (1..=maximo).contains(&pedidos) {
        Ok(pedidos)
    } else {
        Err(HilosInvalidos { pedidos, maximo })
    }
}

// ─── Memoria de un proceso ────────────────────────────────────────────────────

/// RAM que usa ahora mismo un proceso (su "conjunto de trabajo"), en bytes.
pub fn memoria_de_proceso(pid: u32) -> io::Result<u64> {
    #[cfg(windows)]
    {
        windows::memoria_de_proceso(pid)
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        Err(no_soportado())
    }
}

// ─── Procesos en segundo plano ────────────────────────────────────────────────

/// Hace que los procesos que lancemos a partir de ahora **no hereden** nuestra entrada y salida estándar.
///
/// En Windows, un proceso hijo hereda los "tubos" de la consola del padre. Si la CLI lanza el núcleo en segundo
/// plano y este hereda el tubo de salida, la terminal (o un programa que lea la salida de la CLI) se queda
/// esperando a que el núcleo lo cierre, es decir, hasta que el núcleo termine. Pasaba en la Fase 4: `locall list`
/// tardaba 90 s en "acabar".
pub fn no_heredar_entrada_salida() {
    #[cfg(windows)]
    windows::no_heredar_entrada_salida();
}

// ─── Portapapeles ──────────────────────────────────────────────────────────────

/// Pone `texto` en el portapapeles de Windows (lo usa el botón «Copiar» de la app). Solo escribe: Locall nunca lee
/// el portapapeles.
pub fn copiar_al_portapapeles(texto: &str) -> io::Result<()> {
    #[cfg(windows)]
    {
        windows::copiar_al_portapapeles(texto)
    }
    #[cfg(not(windows))]
    {
        let _ = texto;
        Err(no_soportado())
    }
}

// ─── Ctrl+C ───────────────────────────────────────────────────────────────────

/// Se pone a `true` al pulsar Ctrl+C, si se ha llamado a [`capturar_ctrl_c`].
pub(crate) static CTRL_C: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A partir de ahora, Ctrl+C **no cierra el programa**: solo levanta un aviso que se consulta con
/// [`ctrl_c_pulsado`]. Lo usa `locall run` para cortar una respuesta sin salir del chat.
pub fn capturar_ctrl_c() -> io::Result<()> {
    #[cfg(windows)]
    {
        windows::capturar_ctrl_c()
    }
    #[cfg(not(windows))]
    {
        Err(no_soportado())
    }
}

/// ¿Se ha pulsado Ctrl+C desde la última vez que se preguntó? (Al preguntar, el aviso se baja.)
pub fn ctrl_c_pulsado() -> bool {
    CTRL_C.swap(false, std::sync::atomic::Ordering::SeqCst)
}

// ─── Fecha y hora ─────────────────────────────────────────────────────────────

/// Una fecha y hora local (para los registros).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FechaHora {
    pub anio: u16,
    pub mes: u8,
    pub dia: u8,
    pub hora: u8,
    pub minuto: u8,
    pub segundo: u8,
    pub milisegundo: u16,
}

impl FechaHora {
    /// `2026-11-08`
    pub fn fecha(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.anio, self.mes, self.dia)
    }
}

impl fmt::Display for FechaHora {
    /// `2026-11-08 18:42:03.123`
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {:02}:{:02}:{:02}.{:03}",
            self.fecha(),
            self.hora,
            self.minuto,
            self.segundo,
            self.milisegundo
        )
    }
}

/// Fecha y hora local del equipo.
pub fn fecha_hora_local() -> io::Result<FechaHora> {
    #[cfg(windows)]
    {
        Ok(windows::fecha_hora_local())
    }
    #[cfg(not(windows))]
    {
        Err(no_soportado())
    }
}

// ─── Job Object ───────────────────────────────────────────────────────────────

/// Un grupo de procesos que Windows cierra entero cuando se suelta el `Trabajo` **o cuando muere Locall**,
/// aunque sea de golpe (demostrado en la Fase 1: sin él, el motor quedaba huérfano con 1,21 GB).
///
/// Para que funcione, el `Trabajo` debe vivir tanto como los procesos que contiene.
pub struct Trabajo {
    #[cfg(windows)]
    interno: windows::Trabajo,
}

impl Trabajo {
    /// Crea un trabajo vacío con la regla "matar a todos al cerrar".
    pub fn nuevo() -> io::Result<Trabajo> {
        #[cfg(windows)]
        {
            Ok(Trabajo {
                interno: windows::Trabajo::nuevo()?,
            })
        }
        #[cfg(not(windows))]
        {
            Err(no_soportado())
        }
    }

    /// Mete un proceso en el trabajo.
    pub fn asignar(&self, proceso: &Child) -> io::Result<()> {
        #[cfg(windows)]
        {
            self.interno.asignar(proceso)
        }
        #[cfg(not(windows))]
        {
            let _ = proceso;
            Err(no_soportado())
        }
    }
}

#[cfg(not(windows))]
fn no_soportado() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "solo implementado en Windows por ahora (ADR 0004)",
    )
}

// ─── Pruebas ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod pruebas {
    #[test]
    fn da_la_fecha_y_hora_local() {
        let ahora = super::fecha_hora_local().unwrap();
        assert!(
            ahora.anio >= 2026 && (1..=12).contains(&ahora.mes) && (1..=31).contains(&ahora.dia)
        );
        let texto = ahora.to_string();
        assert_eq!(texto.len(), "2026-11-08 18:42:03.123".len(), "{texto}");
        assert!(texto.starts_with(&ahora.fecha()));
    }

    use super::*;

    #[test]
    fn mide_la_memoria_del_equipo() {
        let m = memoria().unwrap();
        assert!(m.total > 1_000_000_000, "más de 1 GB de RAM: {}", m.total);
        assert!(m.disponible > 0 && m.disponible <= m.total);
    }

    #[test]
    fn conoce_el_procesador() {
        let p = procesador();
        assert_eq!(p.hilos_logicos, hilos_logicos());
        assert!(p.hilos_logicos >= 1);
        if let Some(nombre) = &p.nombre {
            assert!(!nombre.trim().is_empty());
        }
        assert_eq!(hilos_para_motor(), p.hilos_logicos);
    }

    #[test]
    fn rechaza_numeros_de_hilos_imposibles() {
        let maximo = hilos_logicos();
        assert_eq!(validar_hilos(1), Ok(1));
        assert_eq!(validar_hilos(maximo), Ok(maximo));
        assert!(validar_hilos(0).is_err());
        assert!(validar_hilos(maximo + 1).is_err());
        assert!(
            validar_hilos(246).is_err() || maximo >= 246,
            "el error de la Fase 0"
        );
    }

    #[test]
    fn mide_la_memoria_de_un_proceso() {
        let bytes = memoria_de_proceso(std::process::id()).unwrap();
        assert!(bytes > 0);
    }

    #[cfg(windows)]
    #[test]
    fn el_trabajo_cierra_sus_procesos_al_soltarlo() {
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        // Un proceso que, por sí solo, tardaría ~30 s en terminar.
        let mut hijo = Command::new("ping")
            .args(["-n", "30", "127.0.0.1"])
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let trabajo = Trabajo::nuevo().unwrap();
        trabajo.asignar(&hijo).unwrap();
        assert!(hijo.try_wait().unwrap().is_none(), "debería seguir vivo");

        drop(trabajo); // soltar el trabajo = Windows cierra sus procesos

        let inicio = Instant::now();
        while hijo.try_wait().unwrap().is_none() {
            assert!(
                inicio.elapsed() < Duration::from_secs(5),
                "el proceso no terminó al soltar el trabajo"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

#[cfg(test)]
mod pruebas_portapapeles {
    /// Escribe en el portapapeles real del usuario: solo a mano (`cargo test -- --ignored`), y comprobándolo desde
    /// fuera (por ejemplo con `Get-Clipboard` en PowerShell).
    #[test]
    #[ignore = "escribe en el portapapeles"]
    fn copia_texto_con_acentos_al_portapapeles() {
        super::copiar_al_portapapeles("Locall: prueba del portapapeles — ñ á 🙂").unwrap();
    }
}
