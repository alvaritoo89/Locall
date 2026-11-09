//! Arranque **bajo demanda** (decisión ratificada): si no hay ningún Locall abierto, la CLI arranca `locall serve`
//! en segundo plano y espera a que responda. Después, el núcleo se cierra solo por inactividad.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use locall_config::{Config, Rutas};

use crate::cliente::{Cliente, ErrorCli, leer_token};

/// Tiempo máximo que se espera a que el núcleo recién arrancado responda.
const ESPERA_ARRANQUE: Duration = Duration::from_secs(20);

/// Un cliente para el Locall que ya está abierto, o `None` si no hay ninguno.
pub fn conectar(rutas: &Rutas) -> Result<Option<Cliente>, ErrorCli> {
    let (config, _) = Config::leer(&rutas.config()).map_err(|e| ErrorCli::Config(e.to_string()))?;
    let base = format!("http://127.0.0.1:{}", config.puerto);
    let Some(token) = leer_token(&rutas.token()) else {
        return Ok(None); // sin token, nunca se ha abierto (o no es nuestro)
    };
    let cliente = Cliente::nuevo(base, token);
    Ok(cliente.responde().then_some(cliente))
}

/// Un cliente para Locall, arrancándolo en segundo plano si no está abierto.
pub fn conectar_o_arrancar(rutas: &Rutas, ejecutable: &Path) -> Result<Cliente, ErrorCli> {
    if let Some(cliente) = conectar(rutas)? {
        return Ok(cliente);
    }
    let mut hijo = lanzar_en_segundo_plano(ejecutable, rutas)?;
    let inicio = Instant::now();
    loop {
        if let Some(cliente) = conectar(rutas)? {
            return Ok(cliente);
        }
        // Si el núcleo terminó (por ejemplo, el puerto lo usa otro programa), no tiene sentido seguir esperando.
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

/// Lanza `locall serve` sin ventana y separado de esta consola: así sigue vivo cuando la CLI termina, y un Ctrl+C en
/// la consola no le llega.
fn lanzar_en_segundo_plano(
    ejecutable: &Path,
    rutas: &Rutas,
) -> Result<std::process::Child, ErrorCli> {
    // Que no herede la salida de esta consola: si no, quien espere a que la CLI termine (la terminal, un script)
    // esperaría también al núcleo.
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
        // DETACHED_PROCESS: sin consola. CREATE_NEW_PROCESS_GROUP: los Ctrl+C de esta consola no le llegan.
        const SEPARADO: u32 = 0x0000_0008 | 0x0000_0200;
        // CREATE_BREAKAWAY_FROM_JOB: si la terminal mete sus procesos en un Job Object que los mata al cerrarse
        // (algunas lo hacen), el núcleo se sale de él. Si el Job no lo permite, se intenta sin salirse.
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
