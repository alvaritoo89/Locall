//! Lo poco que hay que preguntarle directamente a Windows. Es el único sitio del almacén con `unsafe`.

use std::io;
use std::path::Path;

/// Bytes libres disponibles para el usuario actual en el disco donde está `ruta`.
#[cfg(windows)]
pub fn espacio_libre(ruta: &Path) -> io::Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    // Windows quiere la ruta en UTF-16 y terminada en un 0.
    let ancha: Vec<u16> = ruta.as_os_str().encode_wide().chain([0]).collect();
    let mut libres: u64 = 0;
    // SAFETY: `ancha` es una cadena UTF-16 terminada en 0 que vive durante la llamada; `libres` es un u64 válido;
    // los otros dos punteros pueden ser nulos según la documentación de GetDiskFreeSpaceExW.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            ancha.as_ptr(),
            &mut libres,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(libres)
}

#[cfg(not(windows))]
pub fn espacio_libre(_ruta: &Path) -> io::Result<u64> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "solo implementado en Windows por ahora (ADR 0004)",
    ))
}

/// Cuántos nombres (enlaces duros) tiene un archivo. Un blob con 1 solo nombre no lo usa nadie más.
#[cfg(windows)]
pub fn numero_de_enlaces(ruta: &Path) -> io::Result<u32> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };

    let archivo = std::fs::File::open(ruta)?;
    // SAFETY: es una estructura de datos simple (solo números), así que llenarla de ceros es válido.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: el asa (handle) es de un archivo abierto que vive durante la llamada, e `info` es válida.
    let ok = unsafe { GetFileInformationByHandle(archivo.as_raw_handle(), &mut info) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(info.nNumberOfLinks)
}

#[cfg(not(windows))]
pub fn numero_de_enlaces(_ruta: &Path) -> io::Result<u32> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "solo implementado en Windows por ahora (ADR 0004)",
    ))
}
