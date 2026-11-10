//! Recordar el tamaño y la posición de la ventana entre usos.
//!
//! Al cerrar, se guardan en `app-ventana.txt` (en la carpeta de datos de Locall) como una línea de texto:
//! `x y ancho alto maximizada`, en píxeles reales de la pantalla. Al abrir, se recuperan **solo si la barra de título
//! cae dentro de una pantalla conectada**: si se desconectó un monitor, la ventana no aparece fuera de la vista.
//!
//! Sin dependencias: el complemento oficial (`tauri-plugin-window-state`) haría lo mismo con un crate más.

use std::path::PathBuf;

use locall_config::Rutas;
use tauri::{Monitor, PhysicalPosition, PhysicalSize, WebviewWindow, Window};

const ARCHIVO: &str = "app-ventana.txt";

/// Tamaño mínimo razonable (el de `tauri.conf.json`); algo más pequeño en el archivo se ignora.
const MINIMO: (u32, u32) = (600, 400);

/// Dónde y cómo estaba la ventana.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometria {
    pub x: i32,
    pub y: i32,
    pub ancho: u32,
    pub alto: u32,
    pub maximizada: bool,
}

impl Geometria {
    /// Lee la línea del archivo. Cualquier cosa rara (archivo de otra versión, editado a mano…) se ignora.
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

    /// ¿Cae la barra de título (un punto cerca de la esquina superior izquierda) dentro de alguna de estas pantallas
    /// (posición y tamaño de cada una)?
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

/// Coloca la ventana como estaba la última vez (si se puede) y la muestra. Se llama al arrancar.
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

/// Guarda cómo está la ventana. Se llama al cerrarla. Si falla, no pasa nada: la próxima vez se abre en el centro.
pub fn guardar(ventana: &Window) {
    let Some(ruta) = ruta() else { return };
    if ventana.is_minimized().unwrap_or(false) {
        return; // minimizada, la posición no significa nada
    }
    let maximizada = ventana.is_maximized().unwrap_or(false);
    let g = if maximizada {
        // Maximizada: se recuerda que lo estaba, pero se conserva el tamaño normal de la última vez (para cuando
        // se restaure).
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
        let dos = [(0, 0, 1920, 1080), (-2560, 0, 2560, 1440)]; // otra pantalla a la izquierda
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
