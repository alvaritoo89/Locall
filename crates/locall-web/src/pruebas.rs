//! Generator tests, with the official catalog (the one shipped inside Locall) and with a malicious one.

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
    // Even though the catalog is signed, the website does not trust it for HTML.
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
        "firma.html",
        "404.html",
    ] {
        assert!(rutas.contains(&PathBuf::from(fija)), "{fija}");
    }
    assert!(rutas.contains(&PathBuf::from("modelos").join("index.html")));
    for m in &c.modelos {
        assert!(rutas.contains(&PathBuf::from("modelos").join(format!("{}.html", m.nombre))));
    }
    assert_eq!(rutas.len(), 6 + c.modelos.len());
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
    // The author's decision (ADR 0022): «compatible», never «probado» (tested) as a claim.
    assert!(!todo.contains("probado en"));
    assert!(todo.contains("compatible"));
}

#[test]
fn solo_enlaza_a_github_hugging_face_y_signpath() {
    // No fonts, scripts or analytics from other sites. SignPath only appears as plain links in the code signing policy
    // and on the download page (ADR 0035).
    let todo: String = paginas(&oficial()).into_iter().map(|(_, h)| h).collect();
    for (i, _) in todo.match_indices("https://") {
        let resto = &todo[i..];
        assert!(
            resto.starts_with("https://github.com/")
                || resto.starts_with("https://huggingface.co/")
                || resto.starts_with("https://docs.github.com/")
                || resto.starts_with("https://signpath.io/")
                || resto.starts_with("https://signpath.org/"),
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
    // The catalog copy on the website is identical byte for byte (its signature remains valid).
    let copia = std::fs::read(destino.join("catalogo/catalogo.json")).unwrap();
    let firma = std::fs::read_to_string(destino.join("catalogo/catalogo.json.minisig")).unwrap();
    assert!(locall_catalogo::verificar(&copia, &firma, locall_catalogo::CLAVE_PUBLICA).is_ok());
    assert!(escritos.len() > 20);
    let _ = std::fs::remove_dir_all(&destino);
}

#[test]
fn la_descarga_enlaza_al_instalador_de_la_version_publicada() {
    let todo: Vec<(PathBuf, String)> = paginas(&oficial());
    let descargar = &todo
        .iter()
        .find(|(r, _)| r == &PathBuf::from("descargar.html"))
        .unwrap()
        .1;
    let v = crate::VERSION_PUBLICADA;
    assert!(descargar.contains(&format!(
        "https://github.com/alvaritoo89/Locall/releases/download/v{v}/Locall_{v}_x64-setup.exe"
    )));
    assert!(descargar.contains(crate::CLAVE_PUBLICA_MINISIGN));
    assert!(
        descargar.contains("Ejecutar de todas formas"),
        "explica el aviso de Windows"
    );
    assert!(!todo.iter().any(|(_, h)| h.contains("Próximamente")));
    // The published version has its release notes (ADR 0033): a version that was never prepared cannot be linked.
    let notas =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../docs/releases/{v}.md"));
    assert!(notas.is_file(), "faltan {}", notas.display());
}

#[test]
fn la_clave_de_las_versiones_es_la_del_catalogo() {
    // One project key (ADR 0033): the same one the app uses to verify the catalog.
    assert_eq!(
        crate::CLAVE_PUBLICA_MINISIGN,
        locall_catalogo::CLAVE_PUBLICA
    );
}

#[test]
fn la_politica_de_firma_esta_enlazada_en_todas_las_paginas() {
    // SignPath Foundation asks for the code signing policy to be reachable from the home and download pages (ADR 0035).
    for (ruta, html) in paginas(&oficial()) {
        assert!(
            html.contains("firma.html\">Firma de código</a>"),
            "{}",
            ruta.display()
        );
    }
    let todo: Vec<(PathBuf, String)> = paginas(&oficial());
    let firma = &todo
        .iter()
        .find(|(r, _)| r == &PathBuf::from("firma.html"))
        .unwrap()
        .1;
    assert!(firma.contains("Free code signing provided by"));
    assert!(firma.contains("certificate by"));
    assert!(firma.contains("Approvers"));
    let descargar = &todo
        .iter()
        .find(|(r, _)| r == &PathBuf::from("descargar.html"))
        .unwrap()
        .1;
    assert!(descargar.contains("SignPath Foundation"));
}
