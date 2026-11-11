//! **On-demand** startup (confirmed decision): if no Locall is open, the CLI starts `locall serve` in the background
//! and waits for it to answer. Afterwards, the core closes by itself when idle.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use locall_config::{Config, Rutas};

use crate::cliente::{Cliente, ErrorCli, leer_token};

/// Maximum time to wait for the freshly started core to answer.
const ESPERA_ARRANQUE: Duration = Duration::from_secs(20);

/// A client for the Locall that is already open, or `None` if there is none.
pub fn conectar(rutas: &Rutas) -> Result<Option<Cliente>, ErrorCli> {
    let (config, _) = Config::leer(&rutas.config()).map_err(|e| ErrorCli::Config(e.to_string()))?;
    let base = format!("http://127.0.0.1:{}", config.puerto);
    let Some(token) = leer_token(&rutas.token()) else {
        return Ok(None); // no token: it has never been opened (or it is not ours)
    };
    let cliente = Cliente::nuevo(base, token);
    Ok(cliente.responde().then_some(cliente))
}

/// A client for Locall, starting it in the background if it is not open.
pub fn conectar_o_arrancar(rutas: &Rutas, ejecutable: &Path) -> Result<Cliente, ErrorCli> {
    if let Some(cliente) = conectar(rutas)? {
        return Ok(cliente);
    }
    // Tell the user (on the error output, to keep the normal output clean): this way they know Locall stays open for a
    // while and in which folder. (Without this notice, in Phase 4 we opened one in another folder without noticing.)
    {
        use std::io::Write;
        let _ = writeln!(
            std::io::stderr(),
            "(Abriendo Locall en segundo plano, con los datos de {}…)",
            rutas.datos().display()
        );
    }
    let mut hijo = lanzar_en_segundo_plano(ejecutable, rutas)?;
    let inicio = Instant::now();
    loop {
        if let Some(cliente) = conectar(rutas)? {
            return Ok(cliente);
        }
        // If the core exited (for example, another program uses the port), there is no point waiting any longer.
        if let Ok(Some(estado)) = hijo.try_wait() {
            return Err(ErrorCli::Arranque(format!(
                "se cerró al arrancar ({estado}); los detalles están en {}",
                rutas.registros().display()
            )));
        }
        if inicio.elapsed() > ESPERA_ARRANQUE {
            return Err(ErrorCli::Arranque(format!(
                "no respondió en {} s; los detalles están en {}",
                ESPERA_ARRANQUE.as_secs(),
                rutas.registros().display()
            )));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Launches `locall serve` without a window and detached from this console: this way it stays alive when the CLI
/// exits, and a Ctrl+C in the console does not reach it.
fn lanzar_en_segundo_plano(
    ejecutable: &Path,
    rutas: &Rutas,
) -> Result<std::process::Child, ErrorCli> {
    // It must not inherit this console's output: otherwise whoever waits for the CLI to finish (the terminal, a script)
    // would wait for the core too.
    locall_system::no_heredar_entrada_salida();
    let mut orden = Command::new(ejecutable);
    orden
        .arg("serve")
        .env("LOCALL_DATOS", rutas.datos())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS: no console. CREATE_NEW_PROCESS_GROUP: this console's Ctrl+C does not reach it.
        const SEPARADO: u32 = 0x0000_0008 | 0x0000_0200;
        // CREATE_BREAKAWAY_FROM_JOB: if the terminal puts its processes in a Job Object that kills them when it closes
        // (some do), the core breaks away from it. If the Job does not allow it, it is tried without breaking away.
        const FUERA_DEL_JOB: u32 = 0x0100_0000;
        orden.creation_flags(SEPARADO | FUERA_DEL_JOB);
        if let Ok(hijo) = orden.spawn() {
            return Ok(hijo);
        }
        orden.creation_flags(SEPARADO);
    }
    orden
        .spawn()
        .map_err(|e| ErrorCli::Arranque(format!("no se pudo lanzar {}: {e}", ejecutable.display())))
}
