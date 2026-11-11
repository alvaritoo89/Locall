//! The little that has to be asked of Windows directly. It is the only place in the store with `unsafe`.

use std::io;
use std::path::Path;

/// Free bytes available to the current user on the disk where `ruta` is.
#[cfg(windows)]
pub fn espacio_libre(ruta: &Path) -> io::Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    // Windows wants the path in UTF-16 and 0-terminated.
    let ancha: Vec<u16> = ruta.as_os_str().encode_wide().chain([0]).collect();
    let mut libres: u64 = 0;
    // SAFETY: `ancha` is a 0-terminated UTF-16 string that lives during the call; `libres` is a valid u64; the other
    // two pointers may be null according to the GetDiskFreeSpaceExW documentation.
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

/// How many names (hard links) a file has. A blob with a single name is not used by anyone else.
#[cfg(windows)]
pub fn numero_de_enlaces(ruta: &Path) -> io::Result<u32> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };

    let archivo = std::fs::File::open(ruta)?;
    // SAFETY: it is a plain data struct (numbers only), so filling it with zeros is valid.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: the handle belongs to an open file that lives during the call, and `info` is valid.
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
