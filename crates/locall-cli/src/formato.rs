//! Cómo se muestran las cosas en la terminal: tamaños, tiempos y tablas.

/// `1185376` → `1.2 MB`. Unidades decimales (1 GB = 1000 MB), las mismas que usa Hugging Face para los modelos.
pub fn tamano(bytes: u64) -> String {
    const UNIDADES: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut valor = bytes as f64;
    let mut unidad = 0;
    while valor >= 1000.0 && unidad < UNIDADES.len() - 1 {
        valor /= 1000.0;
        unidad += 1;
    }
    if unidad == 0 {
        format!("{bytes} B")
    } else {
        format!("{valor:.1} {}", UNIDADES[unidad])
    }
}

/// Cuánto hace de `momento` (segundos desde 1970) visto desde `ahora`.
pub fn hace(momento: Option<u64>, ahora: u64) -> String {
    let Some(momento) = momento else {
        return "nunca".into();
    };
    let segundos = ahora.saturating_sub(momento);
    match segundos {
        0..60 => "hace un momento".into(),
        60..3600 => format!("hace {} min", segundos / 60),
        3600..86_400 => format!("hace {} h", segundos / 3600),
        86_400..172_800 => "hace 1 día".into(),
        _ => format!("hace {} días", segundos / 86_400),
    }
}

/// Segundos desde 1970 ahora mismo.
pub fn ahora() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Una tabla de texto con columnas alineadas a la izquierda.
pub fn tabla(cabecera: &[&str], filas: &[Vec<String>]) -> String {
    let mut anchos: Vec<usize> = cabecera.iter().map(|c| c.chars().count()).collect();
    for fila in filas {
        for (i, celda) in fila.iter().enumerate() {
            anchos[i] = anchos[i].max(celda.chars().count());
        }
    }
    let linea = |celdas: Vec<&str>| {
        let mut texto = String::new();
        for (i, celda) in celdas.iter().enumerate() {
            if i + 1 == celdas.len() {
                texto.push_str(celda); // la última columna, sin espacios de relleno al final
            } else {
                let relleno = anchos[i] - celda.chars().count();
                texto.push_str(celda);
                texto.push_str(&" ".repeat(relleno + 3));
            }
        }
        texto.push('\n');
        texto
    };
    let mut salida = linea(cabecera.to_vec());
    for fila in filas {
        salida.push_str(&linea(fila.iter().map(String::as_str).collect()));
    }
    salida
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn tamanos() {
        assert_eq!(tamano(512), "512 B");
        assert_eq!(tamano(1_185_376), "1.2 MB");
        assert_eq!(tamano(4_920_000_000), "4.9 GB");
    }

    #[test]
    fn tiempos() {
        let ahora = 1_000_000;
        assert_eq!(hace(None, ahora), "nunca");
        assert_eq!(hace(Some(ahora - 5), ahora), "hace un momento");
        assert_eq!(hace(Some(ahora - 300), ahora), "hace 5 min");
        assert_eq!(hace(Some(ahora - 7200), ahora), "hace 2 h");
        assert_eq!(hace(Some(ahora - 90_000), ahora), "hace 1 día");
        assert_eq!(hace(Some(ahora - 3 * 86_400), ahora), "hace 3 días");
    }

    #[test]
    fn tablas_alineadas() {
        let texto = tabla(
            &["ID", "TAMAÑO"],
            &[
                vec!["qwen".into(), "1.1 GB".into()],
                vec!["a".into(), "5 B".into()],
            ],
        );
        assert_eq!(texto, "ID     TAMAÑO\nqwen   1.1 GB\na      5 B\n");
    }
}
