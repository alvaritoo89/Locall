//! Pruebas del generador, con el catálogo oficial (el que va dentro de Locall) y con uno malicioso.

use std::path::PathBuf;

use crate::{escapar, generar, paginas, tamano};

fn oficial() -> locall_catalogo::Catalogo {
    locall_catalogo::incluido().unwrap()
}

#[test]
fn escapa_todo_lo_que_podria_ser_html() {
    assert_eq!(
        escapar(r#"<script>alert("x")</script> & 'y'"#),
        "&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt; &amp; &#39;y&#39;"
    );
}

#[test]
fn un_texto_malicioso_del_catalogo_no_se_cuela_en_la_web() {
    // Aunque el catálogo vaya firmado, la web no confía en él para el HTML.
    let mut c = oficial();
    c.modelos[0].descripcion = r#"<img src=x onerror="alert(1)">"#.into();
    c.modelos[0].titulo = "<b>Malo</b>".into();
    for (ruta, html) in paginas(&c) {
        assert!(!html.contains("<img src=x"), "{}", ruta.display());
        assert!(!html.contains("<b>Malo</b>"), "{}", ruta.display());
    }
}

#[test]
fn hay_una_pagina_por_modelo_y_todas_las_fijas() {
    let c = oficial();
    let rutas: Vec<PathBuf> = paginas(&c).into_iter().map(|(r, _)| r).collect();
    for fija in [
        "index.html",
        "descargar.html",
        "privacidad.html",
        "404.html",
    ] {
        assert!(rutas.contains(&PathBuf::from(fija)), "{fija}");
    }
    assert!(rutas.contains(&PathBuf::from("modelos").join("index.html")));
    for m in &c.modelos {
        assert!(rutas.contains(&PathBuf::from("modelos").join(format!("{}.html", m.nombre))));
    }
    assert_eq!(rutas.len(), 5 + c.modelos.len());
}

#[test]
fn cada_modelo_lleva_su_comando_y_nunca_dice_probado() {
    let c = oficial();
    let todo: String = paginas(&c).into_iter().map(|(_, h)| h).collect();
    for m in &c.modelos {
        assert!(
            todo.contains(&format!("locall pull {}", m.nombre)),
            "{}",
            m.nombre
        );
    }
    // Decisión del autor (ADR 0022): «compatible», nunca «probado» como afirmación.
    assert!(!todo.contains("probado en"));
    assert!(todo.contains("compatible"));
}

#[test]
fn solo_enlaza_a_github_y_hugging_face() {
    // Ni fuentes, ni scripts, ni estadísticas de otros sitios.
    let todo: String = paginas(&oficial()).into_iter().map(|(_, h)| h).collect();
    for (i, _) in todo.match_indices("https://") {
        let resto = &todo[i..];
        assert!(
            resto.starts_with("https://github.com/")
                || resto.starts_with("https://huggingface.co/")
                || resto.starts_with("https://docs.github.com/"),
            "enlace no permitido: {}",
            &resto[..resto.len().min(80)]
        );
    }
    assert!(!todo.contains("http://"), "nada sin cifrar");
    assert!(!todo.contains("<iframe"));
}

#[test]
fn los_tamanos_se_escriben_en_espanol() {
    assert_eq!(tamano(2_497_280_256), "2,5 GB");
    assert_eq!(tamano(12_109_565_632), "12,1 GB");
    assert_eq!(tamano(850_000_000), "850 MB");
}

#[test]
fn genera_la_web_completa_en_una_carpeta() {
    let destino = std::env::temp_dir().join(format!("locall-web-prueba-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&destino);
    let estaticos = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/estaticos");
    let c = oficial();
    let escritos = generar(
        &c,
        locall_catalogo::CATALOGO_INCLUIDO,
        locall_catalogo::FIRMA_INCLUIDA,
        &estaticos,
        &destino,
    )
    .unwrap();
    for archivo in [
        "index.html",
        "modelos/qwen3-4b.html",
        "estilo.css",
        "web.js",
        "fuentes/inter-latin-wght-normal.woff2",
        "fuentes/LICENCIA-Inter.txt",
        "logo-claro.png",
        "catalogo/catalogo.json",
        "catalogo/catalogo.json.minisig",
    ] {
        assert!(destino.join(archivo).is_file(), "falta {archivo}");
    }
    // La copia del catálogo en la web es idéntica byte a byte (su firma sigue valiendo).
    let copia = std::fs::read(destino.join("catalogo/catalogo.json")).unwrap();
    let firma = std::fs::read_to_string(destino.join("catalogo/catalogo.json.minisig")).unwrap();
    assert!(locall_catalogo::verificar(&copia, &firma, locall_catalogo::CLAVE_PUBLICA).is_ok());
    assert!(escritos.len() > 20);
    let _ = std::fs::remove_dir_all(&destino);
}
