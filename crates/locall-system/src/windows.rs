//! Windows calls. This is the only file in `locall-system` with `unsafe`: each block explains why it is safe.

use std::io;
use std::os::windows::io::AsRawHandle;
use std::process::Child;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, GlobalFree, HANDLE, HANDLE_FLAG_INHERIT,
    INVALID_HANDLE_VALUE, LocalFree, SYSTEMTIME, SetHandleInformation,
};
use windows_sys::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};
use windows_sys::Win32::System::Console::{
    CTRL_BREAK_EVENT, CTRL_C_EVENT, GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE,
    STD_OUTPUT_HANDLE, SetConsoleCtrlHandler,
};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
    JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOB_OBJECT_UILIMIT_DESKTOP, JOB_OBJECT_UILIMIT_DISPLAYSETTINGS, JOB_OBJECT_UILIMIT_EXITWINDOWS,
    JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES, JOB_OBJECT_UILIMIT_READCLIPBOARD,
    JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS, JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
    JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation, SetInformationJobObject,
};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_EXPAND_SZ,
    REG_OPTION_NON_VOLATILE, REG_SZ, REG_VALUE_TYPE, RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW,
    RegGetValueW, RegQueryValueExW, RegSetValueExW,
};
use windows_sys::Win32::System::SystemInformation::{
    GetLocalTime, GlobalMemoryStatusEx, MEMORYSTATUSEX,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, WaitForSingleObject,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
};

use crate::{FechaHora, Memoria};

/// Converts a text to 0-terminated UTF-16, as Windows wants it.
fn ancho(texto: &str) -> Vec<u16> {
    texto.encode_utf16().chain([0]).collect()
}

/// `WAIT_TIMEOUT`: the process has not finished (the 0 ms wait ran out).
const SIGUE_EN_MARCHA: u32 = 0x102;

pub fn proceso_vivo(proceso: &Child) -> bool {
    // SAFETY: the process handle is valid while `proceso` exists (we borrow it); waiting 0 ms only asks whether it has
    // finished, without blocking or changing anything.
    unsafe { WaitForSingleObject(proceso.as_raw_handle() as _, 0) == SIGUE_EN_MARCHA }
}

/// Clipboard format for UTF-16 text (`CF_UNICODETEXT`).
const TEXTO_UNICODE: u32 = 13;

pub fn copiar_al_portapapeles(texto: &str) -> io::Result<()> {
    let utf16 = ancho(texto);
    // Another program may hold the clipboard open for an instant: retry a little.
    let mut abierto = false;
    for _ in 0..10 {
        // SAFETY: a null HWND is valid (the clipboard gets associated with this task).
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
        // SAFETY: the clipboard is opened by us (OpenClipboard succeeded).
        if unsafe { EmptyClipboard() } == 0 {
            return Err(io::Error::last_os_error());
        }
        let bytes = utf16.len() * std::mem::size_of::<u16>();
        // SAFETY: request movable memory of `bytes` bytes; it is checked not to be null.
        let memoria = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) };
        if memoria.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `memoria` is a valid, freshly requested block; GlobalLock gives a pointer to its `bytes` bytes.
        let destino = unsafe { GlobalLock(memoria) }.cast::<u16>();
        if destino.is_null() {
            let error = io::Error::last_os_error();
            // SAFETY: the block is ours and has not been handed to anyone.
            unsafe { GlobalFree(memoria) };
            return Err(error);
        }
        // SAFETY: `destino` points to `utf16.len()` u16 (the block is exactly that size) and does not overlap `utf16`.
        unsafe {
            std::ptr::copy_nonoverlapping(utf16.as_ptr(), destino, utf16.len());
            GlobalUnlock(memoria);
        }
        // SAFETY: the block holds 0-terminated UTF-16 text. If SetClipboardData succeeds, the block belongs to the system
        // and must NOT be freed; if it fails, it is still ours and is freed.
        if unsafe { SetClipboardData(TEXTO_UNICODE, memoria) }.is_null() {
            let error = io::Error::last_os_error();
            // SAFETY: SetClipboardData failed: the block is still ours.
            unsafe { GlobalFree(memoria) };
            return Err(error);
        }
        Ok(())
    })();
    // SAFETY: we opened it above.
    unsafe { CloseClipboard() };
    resultado
}

pub fn memoria() -> io::Result<Memoria> {
    // SAFETY: MEMORYSTATUSEX is a numbers-only struct: filling it with zeros is valid.
    let mut estado: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    estado.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32; // Windows requires the size to be set
    // SAFETY: `estado` is valid, with a correct dwLength, and lives during the call.
    let ok = unsafe { GlobalMemoryStatusEx(&mut estado) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Memoria {
        total: estado.ullTotalPhys,
        disponible: estado.ullAvailPhys,
    })
}

/// Reads the processor name from the Windows registry
/// (`HKLM\HARDWARE\DESCRIPTION\System\CentralProcessor\0\ProcessorNameString`).
pub fn nombre_procesador() -> Option<String> {
    let subclave = ancho(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0");
    let valor = ancho("ProcessorNameString");
    let mut buffer = [0u16; 256];
    let mut bytes = std::mem::size_of_val(&buffer) as u32;
    // SAFETY: the strings are 0-terminated UTF-16 and live during the call; `buffer` has `bytes` bytes; the type pointer
    // may be null according to the RegGetValueW documentation.
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
    // SAFETY: OpenProcess only receives numbers; it returns null on failure (checked below).
    let proceso = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if proceso.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: numbers-only struct, zeros are valid.
    let mut contadores: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    // SAFETY: `proceso` is a valid, freshly opened handle; `contadores` is valid and its size is given.
    let ok = unsafe {
        K32GetProcessMemoryInfo(
            proceso,
            &mut contadores,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )
    };
    let error = io::Error::last_os_error();
    // SAFETY: we close the handle we opened, exactly once.
    unsafe { CloseHandle(proceso) };
    if ok == 0 {
        return Err(error);
    }
    Ok(contadores.WorkingSetSize as u64)
}

/// A DPAPI blob pointing at some bytes (DPAPI only reads it).
fn blob(datos: &[u8]) -> io::Result<CRYPT_INTEGER_BLOB> {
    let longitud = u32::try_from(datos.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "demasiados datos para cifrar"))?;
    Ok(CRYPT_INTEGER_BLOB {
        cbData: longitud,
        pbData: datos.as_ptr().cast_mut(),
    })
}

/// Copies the result DPAPI allocated into a `Vec` and frees DPAPI's memory.
///
/// # Safety
/// `salida` must be a blob filled by a successful CryptProtectData/CryptUnprotectData call.
unsafe fn tomar_salida(salida: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    let datos = if salida.pbData.is_null() || salida.cbData == 0 {
        Vec::new()
    } else {
        // SAFETY: DPAPI guarantees `pbData` points to `cbData` bytes that are ours until LocalFree.
        unsafe { std::slice::from_raw_parts(salida.pbData, salida.cbData as usize) }.to_vec()
    };
    if !salida.pbData.is_null() {
        // SAFETY: DPAPI allocated it with LocalAlloc and hands ownership to us; it is freed exactly once.
        unsafe { LocalFree(salida.pbData.cast()) };
    }
    datos
}

pub fn proteger(datos: &[u8], proposito: &[u8]) -> io::Result<Vec<u8>> {
    let entrada = blob(datos)?;
    let entropia = blob(proposito)?;
    let mut salida = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // SAFETY: the input blobs point to slices that live during the call and DPAPI only reads them; null description,
    // reserved and prompt are allowed; UI_FORBIDDEN means it never shows a window; `salida` is ours to fill.
    let ok = unsafe {
        CryptProtectData(
            &entrada,
            std::ptr::null(),
            &entropia,
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut salida,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the call succeeded, so `salida` was filled by DPAPI.
    Ok(unsafe { tomar_salida(salida) })
}

pub fn desproteger(cifrado: &[u8], proposito: &[u8]) -> io::Result<Vec<u8>> {
    let entrada = blob(cifrado)?;
    let entropia = blob(proposito)?;
    let mut salida = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // SAFETY: as in `proteger`; a null description pointer means we do not want it back.
    let ok = unsafe {
        CryptUnprotectData(
            &entrada,
            std::ptr::null_mut(),
            &entropia,
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut salida,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the call succeeded, so `salida` was filled by DPAPI.
    Ok(unsafe { tomar_salida(salida) })
}

// ─── The user's PATH (HKCU\Environment) ───

/// An open registry key, closed when dropped.
struct Clave(HKEY);

impl Drop for Clave {
    fn drop(&mut self) {
        // SAFETY: the key was opened by `abrir_clave_del_usuario` and is closed exactly once.
        unsafe { RegCloseKey(self.0) };
    }
}

fn error_de_registro(codigo: u32) -> io::Error {
    io::Error::from_raw_os_error(codigo as i32)
}

/// Opens (creating it if needed) `HKEY_CURRENT_USER\<subclave>` to read and write values.
fn abrir_clave_del_usuario(subclave: &str) -> io::Result<Clave> {
    let nombre = ancho(subclave);
    let mut clave: HKEY = std::ptr::null_mut();
    // SAFETY: `nombre` is 0-terminated UTF-16 that lives during the call; null class, security attributes and
    // disposition are allowed; `clave` is ours to fill.
    let codigo = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            nombre.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_QUERY_VALUE | KEY_SET_VALUE,
            std::ptr::null(),
            &mut clave,
            std::ptr::null_mut(),
        )
    };
    if codigo != ERROR_SUCCESS {
        return Err(error_de_registro(codigo));
    }
    Ok(Clave(clave))
}

/// The user's `Path` value in `HKEY_CURRENT_USER\<subclave>` and its type (`REG_EXPAND_SZ` or `REG_SZ`). If it does
/// not exist: empty, `REG_EXPAND_SZ` (what Windows uses for it).
pub fn path_del_usuario(subclave: &str) -> io::Result<(String, REG_VALUE_TYPE)> {
    let clave = abrir_clave_del_usuario(subclave)?;
    let valor = ancho("Path");
    let mut tipo: REG_VALUE_TYPE = 0;
    let mut bytes: u32 = 0;
    // SAFETY: first call with a null buffer only asks for the type and size; all pointers are valid.
    let codigo = unsafe {
        RegQueryValueExW(
            clave.0,
            valor.as_ptr(),
            std::ptr::null(),
            &mut tipo,
            std::ptr::null_mut(),
            &mut bytes,
        )
    };
    if codigo == ERROR_FILE_NOT_FOUND {
        return Ok((String::new(), REG_EXPAND_SZ));
    }
    if codigo != ERROR_SUCCESS {
        return Err(error_de_registro(codigo));
    }
    if tipo != REG_EXPAND_SZ && tipo != REG_SZ {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "el PATH del usuario no es un texto; no se toca",
        ));
    }
    // Room for the value plus a terminator, in u16 units (the value may change between the two calls: the size the
    // second call reports is checked).
    let mut texto = vec![0u16; bytes as usize / 2 + 1];
    let mut capacidad = (texto.len() * 2) as u32;
    // SAFETY: `texto` has `capacidad` bytes; the other pointers are valid.
    let codigo = unsafe {
        RegQueryValueExW(
            clave.0,
            valor.as_ptr(),
            std::ptr::null(),
            &mut tipo,
            texto.as_mut_ptr().cast(),
            &mut capacidad,
        )
    };
    if codigo != ERROR_SUCCESS {
        return Err(error_de_registro(codigo));
    }
    texto.truncate((capacidad as usize / 2).min(texto.len()));
    while texto.last() == Some(&0) {
        texto.pop();
    }
    let texto = String::from_utf16(&texto).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "el PATH del usuario no es texto válido; no se toca",
        )
    })?;
    Ok((texto, tipo))
}

/// Writes the user's `Path` value, keeping its type.
pub fn guardar_path_del_usuario(
    subclave: &str,
    texto: &str,
    tipo: REG_VALUE_TYPE,
) -> io::Result<()> {
    let clave = abrir_clave_del_usuario(subclave)?;
    let valor = ancho("Path");
    let datos = ancho(texto);
    let bytes = u32::try_from(datos.len() * 2)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "PATH demasiado largo"))?;
    // SAFETY: `datos` is 0-terminated UTF-16 of exactly `bytes` bytes (terminator included), alive during the call.
    let codigo = unsafe {
        RegSetValueExW(
            clave.0,
            valor.as_ptr(),
            0,
            tipo,
            datos.as_ptr().cast(),
            bytes,
        )
    };
    if codigo != ERROR_SUCCESS {
        return Err(error_de_registro(codigo));
    }
    Ok(())
}

/// Tells open programs (Explorer, new terminals) that the environment changed, so new terminals see the new PATH.
pub fn avisar_cambio_de_entorno() {
    let seccion = ancho("Environment");
    let mut resultado = 0usize;
    // SAFETY: broadcasting WM_SETTINGCHANGE with a pointer to a 0-terminated UTF-16 string that lives during the
    // call; SMTO_ABORTIFHUNG and a 5 s limit mean a hung window cannot block us.
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            seccion.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            5000,
            &mut resultado,
        )
    };
}

/// Deletes `HKEY_CURRENT_USER\<subclave>` and everything under it (only for the tests' own key).
#[cfg(test)]
pub fn borrar_clave_del_usuario(subclave: &str) {
    let nombre = ancho(subclave);
    // SAFETY: `nombre` is 0-terminated UTF-16 that lives during the call.
    unsafe {
        windows_sys::Win32::System::Registry::RegDeleteTreeW(HKEY_CURRENT_USER, nombre.as_ptr())
    };
}

/// The Job Object (Windows handle). Dropping it closes the handle and Windows terminates its processes.
pub struct Trabajo(HANDLE);

// A Job Object handle can be used from any thread.
unsafe impl Send for Trabajo {}
unsafe impl Sync for Trabajo {}

impl Trabajo {
    pub fn nuevo() -> io::Result<Trabajo> {
        // SAFETY: null pointers are allowed (no security attributes or name).
        let asa = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if asa.is_null() {
            return Err(io::Error::last_os_error());
        }
        let trabajo = Trabajo(asa); // from here on, Drop closes it no matter what

        // Process limits (ADR 0003 and 0028):
        // - KILL_ON_JOB_CLOSE: if Locall dies, even abruptly, Windows kills everything inside.
        // - ACTIVE_PROCESS = 1: only one process may live in the job, so the engine cannot launch other programs
        //   (cmd.exe, PowerShell…): Windows refuses to create them.
        // - DIE_ON_UNHANDLED_EXCEPTION: a crash ends the process at once, without the Windows error dialog.
        // SAFETY: numbers-only struct, zeros are valid.
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
            | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
        info.BasicLimitInformation.ActiveProcessLimit = 1;
        // SAFETY: `asa` is valid; `info` is valid and its exact size is given.
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

        // Desktop limits (ADR 0028): the engine only computes, it has no window. It cannot use the clipboard, other
        // programs' windows or global atoms, nor change display or system settings, nor shut down or log off.
        let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
            UIRestrictionsClass: JOB_OBJECT_UILIMIT_DESKTOP
                | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
                | JOB_OBJECT_UILIMIT_EXITWINDOWS
                | JOB_OBJECT_UILIMIT_GLOBALATOMS
                | JOB_OBJECT_UILIMIT_HANDLES
                | JOB_OBJECT_UILIMIT_READCLIPBOARD
                | JOB_OBJECT_UILIMIT_WRITECLIPBOARD
                | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS,
        };
        // SAFETY: `asa` is valid; `ui` is valid and its exact size is given.
        let ok = unsafe {
            SetInformationJobObject(
                trabajo.0,
                JobObjectBasicUIRestrictions,
                (&ui as *const JOBOBJECT_BASIC_UI_RESTRICTIONS).cast(),
                std::mem::size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(trabajo)
    }

    pub fn asignar(&self, proceso: &Child) -> io::Result<()> {
        // SAFETY: both handles are valid for the duration of the call (`proceso` is borrowed).
        let ok = unsafe { AssignProcessToJobObject(self.0, proceso.as_raw_handle()) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Drop for Trabajo {
    fn drop(&mut self) {
        // SAFETY: we close the handle we created, exactly once.
        unsafe { CloseHandle(self.0) };
    }
}

/// The computer's local date and time (with its configured time zone and daylight saving time).
pub fn fecha_hora_local() -> FechaHora {
    // SAFETY: GetLocalTime only writes to the struct we pass, which is ours and of the right size; it cannot fail.
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

/// Marks this process's standard input and output as **non-inheritable**.
pub fn no_heredar_entrada_salida() {
    for cual in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: GetStdHandle only queries which handle the process has for that channel (it may be null or invalid, and
        // then it is left alone). SetHandleInformation only changes the "inheritable" flag of a handle of ours.
        unsafe {
            let asa = GetStdHandle(cual);
            if !asa.is_null() && asa != INVALID_HANDLE_VALUE {
                SetHandleInformation(asa, HANDLE_FLAG_INHERIT, 0);
            }
        }
    }
}

/// Called by Windows (on one of its threads) when Ctrl+C or Ctrl+Break is pressed in the console.
unsafe extern "system" fn al_pulsar_ctrl_c(tipo: u32) -> windows_sys::core::BOOL {
    if tipo == CTRL_C_EVENT || tipo == CTRL_BREAK_EVENT {
        crate::CTRL_C.store(true, std::sync::atomic::Ordering::SeqCst);
        1 // handled: do not close the program
    } else {
        0 // closing the window, shutting down…: the usual
    }
}

/// Registers the Ctrl+C handler.
pub fn capturar_ctrl_c() -> io::Result<()> {
    // SAFETY: `al_pulsar_ctrl_c` is an `extern "system"` function that lives for the whole program and only touches an
    // `AtomicBool` (safe from any thread, as Windows requires).
    let ok = unsafe { SetConsoleCtrlHandler(Some(al_pulsar_ctrl_c), 1) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
