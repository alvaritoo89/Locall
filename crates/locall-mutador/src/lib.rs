//! # locall-mutador (tests only)
//!
//! Malformed-input testing (*fuzzing*) without dependencies (ADR 0026): **valid** inputs are taken, corrupted on
//! purpose in many ways, and the reader is checked to **always** finish with a controlled result (usually an error),
//! without:
//!
//! - crashing (a `panic`: index out of bounds, overflow, `unwrap` of something that was not there…);
//! - allocating more memory at once than is reasonable (a huge number in a size field must not lead to allocating
//!   gigabytes). This is measured by [`MedidorDeMemoria`], installed as the global allocator in the test program.
//!
//! Everything is **reproducible**: corruption comes from a fixed seed. If an input fails, it is saved to a file and the
//! message says which one and how to repeat it. `LOCALL_BASURA=<n>` changes the number of inputs (few by default, so
//! that `cargo test` stays fast; for a long pass, for example `LOCALL_BASURA=200000`).

use std::alloc::{GlobalAlloc, Layout, System};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Allocations above this are considered a failure (no Locall reader needs that much at once for the test inputs,
/// which are small).
pub const RESERVA_MAXIMA: usize = 16 * 1024 * 1024;

// ─── Memory meter ─────────────────────────────────────────────────────────────

static MAYOR_RESERVA: AtomicUsize = AtomicUsize::new(0);

/// Memory allocator that records the largest allocation requested. Install it like this in the test file:
///
/// ```ignore
/// #[global_allocator]
/// static MEDIDOR: locall_mutador::MedidorDeMemoria = locall_mutador::MedidorDeMemoria;
/// ```
pub struct MedidorDeMemoria;

// SAFETY: delegates everything to the system allocator; it only adds recording a number (without allocating).
unsafe impl GlobalAlloc for MedidorDeMemoria {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        MAYOR_RESERVA.fetch_max(layout.size(), Ordering::Relaxed);
        // SAFETY: the same `layout` we were given; the caller upholds `alloc`'s rules.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        MAYOR_RESERVA.fetch_max(layout.size(), Ordering::Relaxed);
        // SAFETY: same as `alloc`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from this same allocator (which is the system's) with this `layout`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, nuevo: usize) -> *mut u8 {
        MAYOR_RESERVA.fetch_max(nuevo, Ordering::Relaxed);
        // SAFETY: same as `dealloc` and `alloc`.
        unsafe { System.realloc(ptr, layout, nuevo) }
    }
}

// ─── Corrupting inputs ────────────────────────────────────────────────────────

/// Pseudo-random number generator (xorshift64*): fast, dependency-free and reproducible. **Not** suitable for
/// anything security-related; only for choosing how to corrupt.
pub struct Mutador(u64);

impl Mutador {
    pub fn nuevo(semilla: u64) -> Mutador {
        Mutador(semilla.max(1))
    }

    pub fn numero(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A number from `0` to `tope - 1` (`tope` > 0).
    pub fn hasta(&mut self, tope: usize) -> usize {
        (self.numero() % tope as u64) as usize
    }

    /// Corrupts `datos` with 1 to 4 random changes.
    pub fn estropear(&mut self, datos: &mut Vec<u8>) {
        for _ in 0..=self.hasta(4) {
            self.un_cambio(datos);
        }
    }

    fn un_cambio(&mut self, datos: &mut Vec<u8>) {
        if datos.is_empty() {
            datos.push(self.numero() as u8);
            return;
        }
        let pos = self.hasta(datos.len());
        match self.hasta(9) {
            // Flip a bit.
            0 => datos[pos] ^= 1 << self.hasta(8),
            // Replace a byte with any byte.
            1 => datos[pos] = self.numero() as u8,
            // "Dangerous" single-byte values.
            2 => datos[pos] = [0x00, 0xFF, 0x7F, 0x80, b'"', b'\\', b'\n', b'{'][self.hasta(8)],
            // A huge number (or zero) of 4 or 8 bytes where a size or a counter usually goes.
            3 => {
                let ancho = if self.hasta(2) == 0 { 4 } else { 8 };
                let valor: u64 =
                    [u64::MAX, u64::MAX / 2, u32::MAX as u64, 0, 1 << 31, 1 << 40][self.hasta(6)];
                let bytes = valor.to_le_bytes();
                for (i, b) in bytes[..ancho].iter().enumerate() {
                    if let Some(d) = datos.get_mut(pos + i) {
                        *d = *b;
                    }
                }
            }
            // Insert random bytes.
            4 => {
                let n = 1 + self.hasta(16);
                let nuevos: Vec<u8> = (0..n).map(|_| self.numero() as u8).collect();
                datos.splice(pos..pos, nuevos);
            }
            // Remove a chunk.
            5 => {
                let fin = (pos + 1 + self.hasta(32)).min(datos.len());
                datos.drain(pos..fin);
            }
            // Repeat a chunk.
            6 => {
                let fin = (pos + 1 + self.hasta(64)).min(datos.len());
                let trozo = datos[pos..fin].to_vec();
                let donde = self.hasta(datos.len() + 1);
                datos.splice(donde..donde, trozo);
            }
            // Cut off the end.
            7 => datos.truncate(pos),
            // Swap two bytes.
            _ => {
                let otra = self.hasta(datos.len());
                datos.swap(pos, otra);
            }
        }
    }
}

// ─── The test ─────────────────────────────────────────────────────────────────

/// How many inputs to try: `LOCALL_BASURA` or `por_defecto`.
pub fn iteraciones(por_defecto: usize) -> usize {
    std::env::var("LOCALL_BASURA")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(por_defecto)
}

/// Tests `lector` with `iteraciones(por_defecto)` inputs corrupted from `bases`. Fails (saving the input to a file)
/// if any of them makes the reader crash or allocate more than [`RESERVA_MAXIMA`] at once. For the latter to work, the
/// test program must use [`MedidorDeMemoria`].
///
/// The bases themselves and an empty input are tested too.
pub fn probar(
    nombre: &str,
    semilla: u64,
    por_defecto: usize,
    bases: &[Vec<u8>],
    lector: impl Fn(&[u8]),
) {
    assert!(!bases.is_empty());
    let mut m = Mutador::nuevo(semilla);
    let mut casos: Vec<Vec<u8>> = bases.to_vec();
    casos.push(Vec::new());
    let total = iteraciones(por_defecto);
    for i in 0..total + casos.len() {
        let entrada = if i < casos.len() {
            casos[i].clone()
        } else {
            let mut e = bases[m.hasta(bases.len())].clone();
            m.estropear(&mut e);
            e
        };
        MAYOR_RESERVA.store(0, Ordering::Relaxed);
        let resultado = catch_unwind(AssertUnwindSafe(|| lector(&entrada)));
        let reserva = MAYOR_RESERVA.load(Ordering::Relaxed);
        let problema = match resultado {
            Err(_) => Some("se cerró de golpe (panic)".to_owned()),
            Ok(()) if reserva > RESERVA_MAXIMA => Some(format!(
                "reservó {reserva} bytes de una vez (máximo {RESERVA_MAXIMA})"
            )),
            Ok(()) => None,
        };
        if let Some(problema) = problema {
            let archivo = guardar(nombre, i, &entrada);
            panic!(
                "«{nombre}»: la entrada {i} (semilla {semilla}) {problema}. Guardada en {}",
                archivo.display()
            );
        }
    }
}

fn guardar(nombre: &str, i: usize, entrada: &[u8]) -> PathBuf {
    let ruta = std::env::temp_dir().join(format!("locall-basura-{nombre}-{i}.bin"));
    let _ = std::fs::write(&ruta, entrada);
    ruta
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn es_reproducible() {
        let base = b"{\"hola\": [1, 2, 3]}".to_vec();
        let estropear = |semilla| {
            let mut m = Mutador::nuevo(semilla);
            (0..50)
                .map(|_| {
                    let mut e = base.clone();
                    m.estropear(&mut e);
                    e
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(estropear(7), estropear(7));
        assert_ne!(estropear(7), estropear(8));
    }

    #[test]
    #[should_panic(expected = "se cerró de golpe")]
    fn detecta_un_lector_que_se_cierra() {
        probar("prueba-panic", 1, 100, &[b"abc".to_vec()], |e| {
            let _ = e[2]; // fails on short inputs
        });
    }
}
