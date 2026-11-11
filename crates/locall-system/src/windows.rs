//! Llamadas a Windows. Es el único archivo de `locall-system` con `unsafe`: cada bloque explica por qué es seguro.

use std::io;
use std::os::windows::io::AsRawHandle;
use std::process::Child;

use windows_sys::Win32::Foundation::{
    CloseHandle, GlobalFree, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SYSTEMTIME,
    SetHandleInformation,
};
use windows_sys::Win32::System::Console::{
    CTRL_BREAK_EVENT, CTRL_C_EVENT, GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE,
    STD_OUTPUT_HANDLE, SetConsoleCtrlHandler,
};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
use windows_sys::Win32::System::SystemInformation::{
    GetLocalTime, GlobalMemoryStatusEx, MEMORYSTATUSEX,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, WaitForSingleObject,
};

use crate::{FechaHora, Memoria};

/// Convierte un texto a UTF-16 terminado en 0, como lo quiere Windows.
fn ancho(texto: &str) -> Vec<u16> {
    texto.encode_utf16().chain([0]).collect()
}

/// `WAIT_TIMEOUT`: el proceso no ha terminado (la espera de 0 ms se agotó).
const SIGUE_EN_MARCHA: u32 = 0x102;

pub fn proceso_vivo(proceso: &Child) -> bool {
    // SAFETY: el identificador del proceso es válido mientras `proceso` exista (lo tenemos prestado); esperar 0 ms
    // solo pregunta si ha terminado, sin bloquear ni cambiar nada.
    unsafe { WaitForSingleObject(proceso.as_raw_handle() as _, 0) == SIGUE_EN_MARCHA }
}

/// Formato del portapapeles para texto UTF-16 (`CF_UNICODETEXT`).
const TEXTO_UNICODE: u32 = 13;

pub fn copiar_al_portapapeles(texto: &str) -> io::Result<()> {
    let utf16 = ancho(texto);
    // Otro programa puede tener el portapapeles abierto un instante: se reintenta un poco.
    let mut abierto = false;
    for _ in 0..10 {
        // SAFETY: un HWND nulo es válido (el portapapeles queda asociado a esta tarea).
        if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
            abierto = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    if !abierto {
        return Err(io::Error::last_os_error());
    }
    let resultado = (|| {
        // SAFETY: el portapapeles está abierto por nosotros (OpenClipboard tuvo éxito).
        if unsafe { EmptyClipboard() } == 0 {
            return Err(io::Error::last_os_error());
        }
        let bytes = utf16.len() * std::mem::size_of::<u16>();
        // SAFETY: pedir memoria movible de `bytes` bytes; se comprueba que no sea nula.
        let memoria = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) };
        if memoria.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `memoria` es un bloque válido recién pedido; GlobalLock da un puntero a sus `bytes` bytes.
        let destino = unsafe { GlobalLock(memoria) }.cast::<u16>();
        if destino.is_null() {
            let error = io::Error::last_os_error();
            // SAFETY: el bloque es nuestro y no se ha entregado a nadie.
            unsafe { GlobalFree(memoria) };
            return Err(error);
        }
        // SAFETY: `destino` apunta a `utf16.len()` u16 (el bloque mide exactamente eso) y no se solapa con `utf16`.
        unsafe {
            std::ptr::copy_nonoverlapping(utf16.as_ptr(), destino, utf16.len());
            GlobalUnlock(memoria);
        }
        // SAFETY: el bloque contiene texto UTF-16 terminado en 0. Si SetClipboardData tiene éxito, el bloque pasa a
        // ser del sistema y NO se debe liberar; si falla, sigue siendo nuestro y se libera.
        if unsafe { SetClipboardData(TEXTO_UNICODE, memoria) }.is_null() {
            let error = io::Error::last_os_error();
            // SAFETY: SetClipboardData falló: el bloque sigue siendo nuestro.
            unsafe { GlobalFree(memoria) };
            return Err(error);
        }
        Ok(())
    })();
    // SAFETY: lo abrimos nosotros arriba.
    unsafe { CloseClipboard() };
    resultado
}

pub fn memoria() -> io::Result<Memoria> {
    // SAFETY: MEMORYSTATUSEX es una estructura de solo números: llenarla de ceros es válido.
    let mut estado: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    estado.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32; // Windows exige indicar el tamaño
    // SAFETY: `estado` es válida, con dwLength correcto, y vive durante la llamada.
    let ok = unsafe { GlobalMemoryStatusEx(&mut estado) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Memoria {
        total: estado.ullTotalPhys,
        disponible: estado.ullAvailPhys,
    })
}

/// Lee el nombre del procesador del registro de Windows
/// (`HKLM\HARDWARE\DESCRIPTION\System\CentralProcessor\0\ProcessorNameString`).
pub fn nombre_procesador() -> Option<String> {
    let subclave = ancho(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0");
    let valor = ancho("ProcessorNameString");
    let mut buffer = [0u16; 256];
    let mut bytes = std::mem::size_of_val(&buffer) as u32;
    // SAFETY: las cadenas son UTF-16 terminadas en 0 y viven durante la llamada; `buffer` tiene `bytes` bytes;
    // el puntero de tipo puede ser nulo según la documentación de RegGetValueW.
    let resultado = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            subclave.as_ptr(),
            valor.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if resultado != 0 {
        return None;
    }
    let caracteres = (bytes as usize / 2).min(buffer.len());
    let texto = String::from_utf16_lossy(&buffer[..caracteres]);
    let limpio = texto.trim_end_matches('\0').trim().to_string();
    (!limpio.is_empty()).then_some(limpio)
}

pub fn memoria_de_proceso(pid: u32) -> io::Result<u64> {
    // SAFETY: OpenProcess solo recibe números; devuelve nulo si falla (comprobado abajo).
    let proceso = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if proceso.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: estructura de solo números, ceros válidos.
    let mut contadores: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    // SAFETY: `proceso` es un asa válida recién abierta; `contadores` es válida y se indica su tamaño.
    let ok = unsafe {
        K32GetProcessMemoryInfo(
            proceso,
            &mut contadores,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )
    };
    let error = io::Error::last_os_error();
    // SAFETY: cerramos el asa que abrimos, una sola vez.
    unsafe { CloseHandle(proceso) };
    if ok == 0 {
        return Err(error);
    }
    Ok(contadores.WorkingSetSize as u64)
}

/// El Job Object (asa de Windows). Al soltarlo se cierra el asa y Windows termina sus procesos.
pub struct Trabajo(HANDLE);

// El asa de un Job Object se puede usar desde cualquier hilo.
unsafe impl Send for Trabajo {}
unsafe impl Sync for Trabajo {}

impl Trabajo {
    pub fn nuevo() -> io::Result<Trabajo> {
        // SAFETY: punteros nulos permitidos (sin atributos de seguridad ni nombre).
        let asa = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if asa.is_null() {
            return Err(io::Error::last_os_error());
        }
        let trabajo = Trabajo(asa); // a partir de aquí, Drop lo cierra pase lo que pase
        // SAFETY: estructura de solo números, ceros válidos.
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `asa` es válida; `info` es válida y se indica su tamaño exacto.
        let ok = unsafe {
            SetInformationJobObject(
                trabajo.0,
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(trabajo)
    }

    pub fn asignar(&self, proceso: &Child) -> io::Result<()> {
        // SAFETY: ambas asas son válidas mientras dure la llamada (`proceso` está prestado).
        let ok = unsafe { AssignProcessToJobObject(self.0, proceso.as_raw_handle()) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Drop for Trabajo {
    fn drop(&mut self) {
        // SAFETY: cerramos el asa que creamos, una sola vez.
        unsafe { CloseHandle(self.0) };
    }
}

/// Fecha y hora local del equipo (con la zona horaria y el horario de verano que tenga configurados).
pub fn fecha_hora_local() -> FechaHora {
    // SAFETY: GetLocalTime solo escribe en la estructura que le pasamos, que es nuestra y del tamaño correcto;
    // no puede fallar.
    let t: SYSTEMTIME = unsafe {
        let mut t = std::mem::zeroed();
        GetLocalTime(&mut t);
        t
    };
    FechaHora {
        anio: t.wYear,
        mes: t.wMonth as u8,
        dia: t.wDay as u8,
        hora: t.wHour as u8,
        minuto: t.wMinute as u8,
        segundo: t.wSecond as u8,
        milisegundo: t.wMilliseconds,
    }
}

/// Marca la entrada y salida estándar de este proceso como **no heredables**.
pub fn no_heredar_entrada_salida() {
    for cual in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: GetStdHandle solo consulta qué asa tiene el proceso para ese canal (puede ser nula o no válida,
        // y entonces no se toca). SetHandleInformation solo cambia el indicador "heredable" de un asa nuestra.
        unsafe {
            let asa = GetStdHandle(cual);
            if !asa.is_null() && asa != INVALID_HANDLE_VALUE {
                SetHandleInformation(asa, HANDLE_FLAG_INHERIT, 0);
            }
        }
    }
}

/// Lo llama Windows (en un hilo suyo) cuando se pulsa Ctrl+C o Ctrl+Pausa en la consola.
unsafe extern "system" fn al_pulsar_ctrl_c(tipo: u32) -> windows_sys::core::BOOL {
    if tipo == CTRL_C_EVENT || tipo == CTRL_BREAK_EVENT {
        crate::CTRL_C.store(true, std::sync::atomic::Ordering::SeqCst);
        1 // atendido: no cerrar el programa
    } else {
        0 // cerrar la ventana, apagar el equipo…: lo de siempre
    }
}

/// Registra el manejador de Ctrl+C.
pub fn capturar_ctrl_c() -> io::Result<()> {
    // SAFETY: `al_pulsar_ctrl_c` es una función `extern "system"` que vive todo el programa y solo toca un
    // `AtomicBool` (seguro desde cualquier hilo, como exige Windows).
    let ok = unsafe { SetConsoleCtrlHandler(Some(al_pulsar_ctrl_c), 1) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
