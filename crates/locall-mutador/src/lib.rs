//! # locall-mutador (solo para pruebas)
//!
//! Pruebas de «entradas basura» (en inglés, *fuzzing*) sin dependencias (ADR 0026): se cogen entradas **válidas**,
//! se estropean a propósito de muchas maneras y se comprueba que el lector **siempre** termina con un resultado
//! controlado (normalmente un error), sin:
//!
//! - cerrarse de golpe (un `panic`: índice fuera de rango, desbordamiento, `unwrap` de algo que no estaba…);
//! - pedir de una vez más memoria de la razonable (un número enorme en un campo de tamaño no debe llevar a reservar
//!   gigas). Lo mide [`MedidorDeMemoria`], que se instala como reservador global en el programa de pruebas.
//!
//! Todo es **reproducible**: el estropeado sale de una semilla fija. Si una entrada falla, se guarda en un archivo y el
//! mensaje dice cuál y cómo repetirla. `LOCALL_BASURA=<n>` cambia el número de entradas (por defecto, pocas, para que
//! `cargo test` siga siendo rápido; para una pasada larga, por ejemplo `LOCALL_BASURA=200000`).

use std::alloc::{GlobalAlloc, Layout, System};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Reservas por encima de esto se consideran un fallo (ningún lector de Locall necesita tanto de una vez para las
/// entradas de prueba, que son pequeñas).
pub const RESERVA_MAXIMA: usize = 16 * 1024 * 1024;

// ─── Medidor de memoria ───────────────────────────────────────────────────────

static MAYOR_RESERVA: AtomicUsize = AtomicUsize::new(0);

/// Reservador de memoria que apunta la reserva más grande que se ha pedido. Se instala así en el archivo de pruebas:
///
/// ```ignore
/// #[global_allocator]
/// static MEDIDOR: locall_mutador::MedidorDeMemoria = locall_mutador::MedidorDeMemoria;
/// ```
pub struct MedidorDeMemoria;

// SAFETY: delega todo en el reservador del sistema; solo añade apuntar un número (sin reservar nada).
unsafe impl GlobalAlloc for MedidorDeMemoria {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        MAYOR_RESERVA.fetch_max(layout.size(), Ordering::Relaxed);
        // SAFETY: mismo `layout` que nos han dado; las reglas de `alloc` las cumple quien llama.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        MAYOR_RESERVA.fetch_max(layout.size(), Ordering::Relaxed);
        // SAFETY: igual que `alloc`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` lo dio este mismo reservador (que es el del sistema) con este `layout`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, nuevo: usize) -> *mut u8 {
        MAYOR_RESERVA.fetch_max(nuevo, Ordering::Relaxed);
        // SAFETY: igual que `dealloc` y `alloc`.
        unsafe { System.realloc(ptr, layout, nuevo) }
    }
}

// ─── Estropear entradas ───────────────────────────────────────────────────────

/// Generador de números pseudoaleatorios (xorshift64*): rápido, sin dependencias y reproducible. **No** sirve para
/// nada de seguridad; solo para elegir cómo estropear.
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

    /// Un número de `0` a `tope - 1` (`tope` > 0).
    pub fn hasta(&mut self, tope: usize) -> usize {
        (self.numero() % tope as u64) as usize
    }

    /// Estropea `datos` con 1 a 4 cambios al azar.
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
            // Cambiar un bit.
            0 => datos[pos] ^= 1 << self.hasta(8),
            // Cambiar un byte por cualquiera.
            1 => datos[pos] = self.numero() as u8,
            // Valores «peligrosos» de un byte.
            2 => datos[pos] = [0x00, 0xFF, 0x7F, 0x80, b'"', b'\\', b'\n', b'{'][self.hasta(8)],
            // Un número enorme (o cero) de 4 u 8 bytes donde suele ir un tamaño o un contador.
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
            // Meter bytes al azar.
            4 => {
                let n = 1 + self.hasta(16);
                let nuevos: Vec<u8> = (0..n).map(|_| self.numero() as u8).collect();
                datos.splice(pos..pos, nuevos);
            }
            // Quitar un trozo.
            5 => {
                let fin = (pos + 1 + self.hasta(32)).min(datos.len());
                datos.drain(pos..fin);
            }
            // Repetir un trozo.
            6 => {
                let fin = (pos + 1 + self.hasta(64)).min(datos.len());
                let trozo = datos[pos..fin].to_vec();
                let donde = self.hasta(datos.len() + 1);
                datos.splice(donde..donde, trozo);
            }
            // Cortar el final.
            7 => datos.truncate(pos),
            // Intercambiar dos bytes.
            _ => {
                let otra = self.hasta(datos.len());
                datos.swap(pos, otra);
            }
        }
    }
}

// ─── La prueba ────────────────────────────────────────────────────────────────

/// Cuántas entradas probar: `LOCALL_BASURA` o `por_defecto`.
pub fn iteraciones(por_defecto: usize) -> usize {
    std::env::var("LOCALL_BASURA")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(por_defecto)
}

/// Prueba `lector` con `iteraciones(por_defecto)` entradas estropeadas a partir de `bases`. Falla (con la entrada
/// guardada en un archivo) si alguna hace que el lector se cierre de golpe o reserve más de [`RESERVA_MAXIMA`] de una
/// vez. Para que lo segundo funcione, el programa de pruebas debe usar [`MedidorDeMemoria`].
///
/// También se prueban las bases tal cual y una entrada vacía.
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
            let _ = e[2]; // falla con las entradas cortas
        });
    }
}
