//! The repository's official catalog (`catalogo/catalogo.json`): it must be valid and its signature must be valid.

const CATALOGO: &[u8] = include_bytes!("../../../catalogo/catalogo.json");

#[test]
fn el_catalogo_del_repositorio_es_valido() {
    let c = locall_catalogo::leer(CATALOGO).unwrap();
    assert!(!c.modelos.is_empty());
    for m in &c.modelos {
        let v = m.por_defecto();
        assert!(v.bytes() > 0, "{}", m.nombre);
        assert_eq!(
            v.motor_comprobado, "b12189",
            "{}: comprobado con el motor que lleva Locall",
            m.nombre
        );
    }
    // What people will type in `locall pull`.
    assert!(c.resolver("qwen3-4b").is_some());
}

#[test]
fn el_catalogo_incluido_tiene_una_firma_oficial_buena() {
    // If this fails, the catalog was edited after signing it: it must be signed again (ADR 0022).
    let c = locall_catalogo::incluido().unwrap();
    assert_eq!(c, locall_catalogo::leer(CATALOGO).unwrap());
    assert_eq!(c.modelos.len(), 12);
}
