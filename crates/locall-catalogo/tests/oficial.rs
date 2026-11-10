//! El catálogo oficial del repositorio (`catalogo/catalogo.json`): que sea válido y que su firma sea buena.

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
    // Lo que escribirá la gente en `locall pull`.
    assert!(c.resolver("qwen3-4b").is_some());
}

#[test]
fn el_catalogo_incluido_tiene_una_firma_oficial_buena() {
    // Si esto falla, el catálogo se editó después de firmarlo: hay que volver a firmarlo (ADR 0022).
    let c = locall_catalogo::incluido().unwrap();
    assert_eq!(c, locall_catalogo::leer(CATALOGO).unwrap());
    assert_eq!(c.modelos.len(), 12);
}
