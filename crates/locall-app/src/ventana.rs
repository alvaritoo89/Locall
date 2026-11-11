//! Remembering the window's size and position between uses.
//!
//! On close they are saved in `app-ventana.txt` (in Locall's data folder) as one line of text:
//! `x y width height maximized`, in real screen pixels. On open they are restored **only if the title bar falls
//! within a connected screen**: if a monitor was disconnected, the window does not appear out of view.
//!
//! No dependencies: the official plugin (`tauri-plugin-window-state`) would do the same with one more crate.

use std::path::PathBuf;

use locall_config::Rutas;
use tauri::{Monitor, PhysicalPosition, PhysicalSize, WebviewWindow, Window};

const ARCHIVO: &str = "app-ventana.txt";

/// Reasonable minimum size (the one in `tauri.conf.json`); anything smaller in the file is ignored.
const MINIMO: (u32, u32) = (600, 400);

/// Where and how the window was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometria {
    pub x: i32,
    pub y: i32,
    pub ancho: u32,
    pub alto: u32,
    pub maximizada: bool,
}

impl Geometria {
    /// Reads the file's line. Anything odd (a file from another version, edited by hand…) is ignored.
    pub fn leer(texto: &str) -> Option<Geometria> {
        let mut partes = texto.split_whitespace();
        let g = Geometria {
            x: partes.next()?.parse().ok()?,
            y: partes.next()?.parse().ok()?,
            ancho: partes.next()?.parse().ok()?,
            alto: partes.next()?.parse().ok()?,
            maximizada: match partes.next()? {
                "1" => true,
                "0" => false,
                _ => return None,
            },
        };
        (partes.next().is_none()
            && g.ancho >= MINIMO.0
            && g.alto >= MINIMO.1
            && g.ancho <= 32_000
            && g.alto <= 32_000)
            .then_some(g)
    }

    pub fn escribir(&self) -> String {
        format!(
            "{} {} {} {} {}\n",
            self.x,
            self.y,
            self.ancho,
            self.alto,
            u8::from(self.maximizada)
        )
    }

    /// Does the title bar (a point near the top-left corner) fall within any of these screens (position and size of
    /// each)?
    pub fn se_ve_en(&self, pantallas: &[(i32, i32, u32, u32)]) -> bool {
        let (px, py) = (i64::from(self.x) + 80, i64::from(self.y) + 16);
        pantallas.iter().any(|&(x, y, ancho, alto)| {
            let (x, y) = (i64::from(x), i64::from(y));
            px >= x && px < x + i64::from(ancho) && py >= y && py < y + i64::from(alto)
        })
    }
}

fn ruta() -> Option<PathBuf> {
    Rutas::por_defecto().ok().map(|r| r.datos().join(ARCHIVO))
}

fn guardada() -> Option<Geometria> {
    Geometria::leer(&std::fs::read_to_string(ruta()?).ok()?)
}

fn pantallas(monitores: &[Monitor]) -> Vec<(i32, i32, u32, u32)> {
    monitores
        .iter()
        .map(|m| {
            (
                m.position().x,
                m.position().y,
                m.size().width,
                m.size().height,
            )
        })
        .collect()
}

/// Places the window as it was last time (if possible) and shows it. Called at startup.
pub fn restaurar(ventana: &WebviewWindow) {
    if let Some(g) = guardada() {
        let monitores = ventana.available_monitors().unwrap_or_default();
        if g.se_ve_en(&pantallas(&monitores)) {
            let _ = ventana.set_size(PhysicalSize::new(g.ancho, g.alto));
            let _ = ventana.set_position(PhysicalPosition::new(g.x, g.y));
            if g.maximizada {
                let _ = ventana.maximize();
            }
        }
    }
    let _ = ventana.show();
    let _ = ventana.set_focus();
}

/// Saves the window's state. Called when it closes. If it fails, nothing happens: next time it opens centered.
pub fn guardar(ventana: &Window) {
    let Some(ruta) = ruta() else { return };
    if ventana.is_minimized().unwrap_or(false) {
        return; // minimized, the position means nothing
    }
    let maximizada = ventana.is_maximized().unwrap_or(false);
    let g = if maximizada {
        // Maximized: remember that it was, but keep last time's normal size (for when it is restored).
        match guardada() {
            Some(anterior) => Geometria {
                maximizada: true,
                ..anterior
            },
            None => match actual(ventana) {
                Some(g) => Geometria {
                    maximizada: true,
                    ..g
                },
                None => return,
            },
        }
    } else {
        match actual(ventana) {
            Some(g) => g,
            None => return,
        }
    };
    let _ = std::fs::write(ruta, g.escribir());
}

fn actual(ventana: &Window) -> Option<Geometria> {
    let posicion = ventana.outer_position().ok()?;
    let tamano = ventana.inner_size().ok()?;
    Some(Geometria {
        x: posicion.x,
        y: posicion.y,
        ancho: tamano.width,
        alto: tamano.height,
        maximizada: false,
    })
}

#[cfg(test)]
mod pruebas {
    use super::Geometria;

    #[test]
    fn se_guarda_y_se_lee_igual() {
        let g = Geometria {
            x: -1200,
            y: 40,
            ancho: 1280,
            alto: 800,
            maximizada: true,
        };
        assert_eq!(g.escribir(), "-1200 40 1280 800 1\n");
        assert_eq!(Geometria::leer(&g.escribir()), Some(g));
    }

    #[test]
    fn lo_raro_se_ignora() {
        for texto in [
            "",
            "1 2 3",
            "a b c d 0",
            "0 0 1280 800 2",
            "0 0 1280 800 0 extra",
            "0 0 10 10 0",
        ] {
            assert_eq!(Geometria::leer(texto), None, "{texto:?}");
        }
    }

    #[test]
    fn solo_se_recupera_si_se_ve_en_una_pantalla() {
        let g = |x, y| Geometria {
            x,
            y,
            ancho: 1280,
            alto: 800,
            maximizada: false,
        };
        let una = [(0, 0, 1920, 1080)];
        let dos = [(0, 0, 1920, 1080), (-2560, 0, 2560, 1440)]; // another screen to the left
        assert!(g(100, 100).se_ve_en(&una));
        assert!(
            !g(-1500, 100).se_ve_en(&una),
            "en una pantalla que ya no está"
        );
        assert!(g(-1500, 100).se_ve_en(&dos));
        assert!(!g(100, 2000).se_ve_en(&dos), "por debajo de todas");
        assert!(
            !g(1900, 100).se_ve_en(&una),
            "la barra de título se saldría por la derecha"
        );
    }
}
