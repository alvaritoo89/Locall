//! The download progress bar.
//!
//! - In a terminal, a single line that keeps being redrawn (`\r`): bar, percentage, size, speed and time left.
//! - If the output is not a terminal (a file, another program), one line every 10 %, so it is not filled with junk.
//!
//! It is written to the error output (stderr), so the normal output (stdout) only carries the result.

use std::io::{IsTerminal, Write};
use std::time::Instant;

use crate::formato;

const ANCHO: usize = 24;

pub struct Barra {
    inicio: Instant,
    /// Bytes already present at the start (when continuing a download): they do not count towards the speed.
    base: Option<u64>,
    terminal: bool,
    ultimo_decimo: Option<u64>,
    dibujada: bool,
}

impl Barra {
    pub fn nueva() -> Barra {
        Barra {
            inicio: Instant::now(),
            base: None,
            terminal: std::io::stderr().is_terminal(),
            ultimo_decimo: None,
            dibujada: false,
        }
    }

    pub fn actualizar(&mut self, descargados: u64, total: u64, parte: u64, partes: u64) {
        let base = *self.base.get_or_insert(descargados);
        if base > 0 && !self.dibujada {
            let _ = writeln!(
                std::io::stderr(),
                "Continuando una descarga anterior desde el {} %",
                porcentaje(base, total)
            );
        }
        self.dibujada = true;
        let linea = linea(
            descargados,
            total,
            parte,
            partes,
            base,
            self.inicio.elapsed().as_secs_f64(),
        );
        let mut error = std::io::stderr();
        if self.terminal {
            let _ = write!(error, "\r{linea:<79}");
            let _ = error.flush();
        } else {
            let decimo = porcentaje(descargados, total) / 10;
            if self.ultimo_decimo != Some(decimo) {
                self.ultimo_decimo = Some(decimo);
                let _ = writeln!(error, "{linea}");
            }
        }
    }

    /// Ends the bar's line (so what comes next is not written over it).
    pub fn terminar(&mut self) {
        if self.terminal && self.dibujada {
            let _ = writeln!(std::io::stderr());
        }
        self.dibujada = false;
    }
}

impl Drop for Barra {
    fn drop(&mut self) {
        self.terminar();
    }
}

fn porcentaje(parte: u64, total: u64) -> u64 {
    // With no known total (0), it is considered complete.
    (parte.min(total) * 100).checked_div(total).unwrap_or(100)
}

/// A progress line, independent of the clock (so it can be tested).
pub fn linea(
    descargados: u64,
    total: u64,
    parte: u64,
    partes: u64,
    base: u64,
    segundos: f64,
) -> String {
    let llenos = if total == 0 {
        ANCHO
    } else {
        ((descargados.min(total) as f64 / total as f64) * ANCHO as f64) as usize
    };
    let barra = format!("{}{}", "█".repeat(llenos), "░".repeat(ANCHO - llenos));
    let mut texto = String::new();
    if partes > 1 {
        texto.push_str(&format!("Parte {parte}/{partes} "));
    }
    texto.push_str(&format!(
        "{barra} {:>3} %  {} de {}",
        porcentaje(descargados, total),
        formato::tamano(descargados),
        formato::tamano(total)
    ));
    let nuevos = descargados.saturating_sub(base);
    if segundos >= 1.0 && nuevos > 0 && descargados < total {
        let velocidad = nuevos as f64 / segundos;
        let quedan = (total - descargados) as f64 / velocidad;
        texto.push_str(&format!(
            "  {}/s  quedan {}",
            formato::tamano(velocidad as u64),
            duracion(quedan as u64)
        ));
    }
    texto
}

/// `75` → `1 min 15 s`
pub fn duracion(segundos: u64) -> String {
    match segundos {
        0..60 => format!("{segundos} s"),
        60..3600 => format!("{} min {} s", segundos / 60, segundos % 60),
        _ => format!("{} h {} min", segundos / 3600, (segundos % 3600) / 60),
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn dibuja_la_linea() {
        assert_eq!(
            linea(500_000_000, 1_000_000_000, 1, 1, 0, 10.0),
            "████████████░░░░░░░░░░░░  50 %  500.0 MB de 1.0 GB  50.0 MB/s  quedan 10 s"
        );
        // At the start, no speed yet; with several parts, it says which one.
        assert_eq!(
            linea(0, 2_000, 2, 3, 0, 0.0),
            "Parte 2/3 ░░░░░░░░░░░░░░░░░░░░░░░░   0 %  0 B de 2.0 KB"
        );
        // When continuing, the speed only counts the new data.
        assert!(linea(600, 1000, 1, 1, 500, 1.0).contains("100 B/s"));
    }

    #[test]
    fn duraciones() {
        assert_eq!(duracion(42), "42 s");
        assert_eq!(duracion(75), "1 min 15 s");
        assert_eq!(duracion(7_260), "2 h 1 min");
    }
}
