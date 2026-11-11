//! Entradas basura contra el catálogo y su firma (ADR 0026): lo que llega de GitHub (o de quien se haga pasar por
//! él) nunca debe cerrar Locall de golpe ni hacerle reservar memoria sin límite.

use locall_catalogo::{CATALOGO_INCLUIDO, CLAVE_PUBLICA, FIRMA_INCLUIDA, leer, verificar};

#[global_allocator]
static MEDIDOR: locall_mutador::MedidorDeMemoria = locall_mutador::MedidorDeMemoria;

#[test]
fn el_lector_del_catalogo_aguanta_entradas_basura() {
    let bases = vec![
        CATALOGO_INCLUIDO.to_vec(),
        include_bytes!("datos/catalogo.json").to_vec(),
    ];
    locall_mutador::probar("catalogo", 0xCA7A_1060, 5_000, &bases, |entrada| {
        let _ = leer(entrada);
        // Un catálogo estropeado nunca pasa la firma.
        assert!(
            entrada == CATALOGO_INCLUIDO
                || verificar(entrada, FIRMA_INCLUIDA, CLAVE_PUBLICA).is_err()
        );
    });
}

#[test]
fn la_comprobacion_de_la_firma_aguanta_firmas_basura() {
    let bases = vec![FIRMA_INCLUIDA.as_bytes().to_vec()];
    locall_mutador::probar("firma", 0xF1_2A, 5_000, &bases, |entrada| {
        let firma = String::from_utf8_lossy(entrada);
        let resultado = verificar(CATALOGO_INCLUIDO, &firma, CLAVE_PUBLICA);
        // Solo vale la firma de verdad: las líneas 2 a 4 (firma, comentario firmado y firma global) deben ser las
        // mismas. Lo demás no cuenta: la primera línea es un comentario sin firmar, y el verificador ignora las líneas
        // que sobren al final (lo descubrió esta prueba; no es un problema: la firma es idéntica).
        if resultado.is_ok() {
            fn firmadas(t: &str) -> Vec<&str> {
                t.lines().skip(1).take(3).map(str::trim).collect()
            }
            assert_eq!(
                firmadas(&firma),
                firmadas(FIRMA_INCLUIDA),
                "aceptó una firma cambiada"
            );
        }
    });
}

#[test]
fn la_clave_publica_aguanta_claves_basura() {
    let bases = vec![CLAVE_PUBLICA.as_bytes().to_vec()];
    locall_mutador::probar("clave", 0xC1A7E, 5_000, &bases, |entrada| {
        let clave = String::from_utf8_lossy(entrada);
        // Solo vale nuestra clave. Las 2 primeras letras solo codifican la marca de algoritmo («Ed»), que el
        // verificador no usa (lo descubrió esta prueba): cambiarlas deja la MISMA clave y el mismo identificador, así
        // que se acepta. Todo lo demás (identificador y clave) debe ser idéntico.
        if verificar(CATALOGO_INCLUIDO, FIRMA_INCLUIDA, &clave).is_ok() {
            let clave = clave.trim();
            assert_eq!(
                clave.get(2..),
                CLAVE_PUBLICA.get(2..),
                "aceptó otra clave: {clave}"
            );
        }
    });
}
