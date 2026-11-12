//! # locall-system
//!
//! What Locall needs to know about the computer and the operating system:
//!
//! - [`memoria`]: total and **truly** available RAM (the memory rule does not trust what Vulkan reports on integrated
//!   GPUs, ADR 0007).
//! - [`procesador`] and [`hilos_para_motor`]: processor name and threads to use (decision D5), with validation
//!   ([`validar_hilos`]) so the Phase 0 bug of the 246 threads never happens again.
//! - [`memoria_de_proceso`]: how much RAM a process (for example, the engine) uses, as measured by the system.
//! - [`Trabajo`]: the Windows Job Object that guarantees the engine dies if Locall dies (ADR 0003).
//!
//! All communication with Windows (`unsafe`) lives in the private `windows` module.

use std::fmt;
use std::io;
use std::process::Child;

#[cfg(windows)]
mod windows;

// ─── Computer memory ──────────────────────────────────────────────────────────

/// The computer's RAM, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Memoria {
    pub total: u64,
    /// What can be used right now without Windows having to move things to disk.
    pub disponible: u64,
}

/// Queries total and available RAM.
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

// ─── Processor and threads ────────────────────────────────────────────────────

/// The computer's processor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Procesador {
    /// Commercial name, for example `Intel(R) Core(TM) …` or `AMD Ryzen …`, if it can be found out.
    pub nombre: Option<String>,
    /// Threads it can run at once.
    pub hilos_logicos: usize,
}

/// Processor information.
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

/// Logical threads of the computer (at least 1).
pub fn hilos_logicos() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

/// Threads Locall gives the engine by default: all the logical ones (Phase 0 decision D5: prompt processing
/// scales with the threads and generation does not get worse).
pub fn hilos_para_motor() -> usize {
    hilos_logicos()
}

/// Error when asking for an impossible number of threads.
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

/// Checks that a requested number of threads (by the user or the configuration) is possible on this computer.
///
/// In Phase 0, a bug made the engine start with 246 threads on a machine with far fewer: generation speed collapsed.
/// This validation prevents it.
pub fn validar_hilos(pedidos: usize) -> Result<usize, HilosInvalidos> {
    let maximo = hilos_logicos();
    if (1..=maximo).contains(&pedidos) {
        Ok(pedidos)
    } else {
        Err(HilosInvalidos { pedidos, maximo })
    }
}

// ─── Process memory ───────────────────────────────────────────────────────────

/// RAM a process is using right now (its "working set"), in bytes.
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

// ─── Background processes ─────────────────────────────────────────────────────

/// Makes the processes we launch from now on **not inherit** our standard input and output.
///
/// On Windows, a child process inherits the parent's console "pipes". If the CLI launches the core in the background
/// and it inherits the output pipe, the terminal (or a program reading the CLI's output) keeps waiting for the core
/// to close it, that is, until the core exits. It happened in Phase 4: `locall list` took 90 s to "finish".
pub fn no_heredar_entrada_salida() {
    #[cfg(windows)]
    windows::no_heredar_entrada_salida();
}

// ─── Processes ────────────────────────────────────────────────────────────────

/// Is this process still running? Asks Windows without waiting, and without needing exclusive access to the process
/// (unlike `Child::try_wait`), so it can be used with an engine shared between threads.
pub fn proceso_vivo(proceso: &std::process::Child) -> bool {
    #[cfg(windows)]
    {
        windows::proceso_vivo(proceso)
    }
    #[cfg(not(windows))]
    {
        let _ = proceso;
        true
    }
}

// ─── Clipboard ────────────────────────────────────────────────────────────────

/// Puts `texto` on the Windows clipboard (used by the app's «Copiar» button). It only writes: Locall never reads
/// the clipboard.
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

/// Set to `true` when Ctrl+C is pressed, if [`capturar_ctrl_c`] has been called.
pub(crate) static CTRL_C: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// From now on, Ctrl+C **does not close the program**: it only raises a flag that is checked with
/// [`ctrl_c_pulsado`]. Used by `locall run` to cut an answer short without leaving the chat.
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

/// Has Ctrl+C been pressed since the last time we asked? (Asking lowers the flag.)
pub fn ctrl_c_pulsado() -> bool {
    CTRL_C.swap(false, std::sync::atomic::Ordering::SeqCst)
}

// ─── Date and time ────────────────────────────────────────────────────────────

/// A local date and time (for the logs).
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

/// The computer's local date and time.
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

/// A group of processes that Windows closes entirely when the `Trabajo` is dropped **or when Locall dies**, even
/// abruptly (shown in Phase 1: without it, the engine was left orphaned holding over 1 GB).
///
/// For it to work, the `Trabajo` must live as long as the processes it contains.
pub struct Trabajo {
    #[cfg(windows)]
    interno: windows::Trabajo,
}

impl Trabajo {
    /// Creates an empty job with Locall's rules (ADR 0003 and 0028): everything inside dies when the job is closed
    /// (or when Locall dies), only **one process** may live in it (it cannot launch other programs), a crash ends it
    /// without the Windows error dialog, and it cannot touch the desktop (clipboard, other windows, system settings).
    ///
    /// It is meant for **one** process: a second one assigned to it would be refused.
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

    /// Puts a process into the job.
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

// ─── Tests ────────────────────────────────────────────────────────────────────

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

        // A process that, on its own, would take ~30 s to finish.
        let mut hijo = Command::new("ping")
            .args(["-n", "30", "127.0.0.1"])
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let trabajo = Trabajo::nuevo().unwrap();
        trabajo.asignar(&hijo).unwrap();
        assert!(hijo.try_wait().unwrap().is_none(), "debería seguir vivo");

        drop(trabajo); // dropping the job = Windows closes its processes

        let inicio = Instant::now();
        while hijo.try_wait().unwrap().is_none() {
            assert!(
                inicio.elapsed() < Duration::from_secs(5),
                "el proceso no terminó al soltar el trabajo"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// `cmd.exe` that first waits for a line on its input (`set /p`, internal: it launches nothing) and then tries to
    /// launch another `cmd.exe` that writes «hijo». Waiting lets the test put it in the job before it tries.
    /// Returns what it wrote to its output.
    #[cfg(windows)]
    fn lanza_un_hijo(trabajo: Option<&Trabajo>) -> String {
        use std::io::{Read, Write};
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};

        let mut padre = Command::new("cmd.exe")
            .raw_arg("/d /c \"set /p x= & cmd.exe /d /c echo hijo\"")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        if let Some(trabajo) = trabajo {
            trabajo.asignar(&padre).unwrap();
        }
        padre.stdin.take().unwrap().write_all(b"\r\n").unwrap();
        let mut salida = String::new();
        padre
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut salida)
            .unwrap();
        padre.wait().unwrap();
        salida
    }

    #[cfg(windows)]
    #[test]
    fn dentro_del_trabajo_no_se_pueden_lanzar_otros_programas() {
        // Control: outside the job, the second cmd.exe does run (so the test really checks something).
        assert!(lanza_un_hijo(None).contains("hijo"));

        let trabajo = Trabajo::nuevo().unwrap();
        let salida = lanza_un_hijo(Some(&trabajo));
        assert!(
            !salida.contains("hijo"),
            "el proceso del trabajo pudo lanzar otro programa: {salida:?}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn un_trabajo_solo_admite_un_proceso() {
        use std::process::{Command, Stdio};

        let lanzar = || {
            Command::new("ping")
                .args(["-n", "30", "127.0.0.1"])
                .stdout(Stdio::null())
                .spawn()
                .unwrap()
        };
        let mut primero = lanzar();
        let mut segundo = lanzar();
        let trabajo = Trabajo::nuevo().unwrap();
        trabajo.asignar(&primero).unwrap();
        assert!(
            trabajo.asignar(&segundo).is_err(),
            "admitió un segundo proceso"
        );
        let _ = segundo.kill();
        let _ = segundo.wait();
        drop(trabajo);
        let _ = primero.wait();
    }
}

#[cfg(test)]
mod pruebas_portapapeles {
    /// Writes to the user's real clipboard: only by hand (`cargo test -- --ignored`), checking it from outside (for
    /// example with `Get-Clipboard` in PowerShell).
    #[test]
    #[ignore = "escribe en el portapapeles"]
    fn copia_texto_con_acentos_al_portapapeles() {
        super::copiar_al_portapapeles("Locall: prueba del portapapeles — ñ á 🙂").unwrap();
    }
}
