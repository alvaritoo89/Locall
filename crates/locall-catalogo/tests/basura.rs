//! Malformed inputs against the catalog and its signature (ADR 0026): what arrives from GitHub (or from someone
//! impersonating it) must never crash Locall or make it allocate memory without limit.

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
        // A corrupted catalog never passes the signature check.
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
        // Only the real signature is valid: lines 2 to 4 (signature, trusted comment and global signature) must be the
        // same. The rest does not count: the first line is an unsigned comment, and the verifier ignores extra lines at the
        // end (this test discovered it; not a problem: the signature is identical).
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
        // Only our key is valid. The first 2 letters only encode the algorithm marker («Ed»), which the verifier does not
        // use (this test discovered it): changing them leaves the SAME key and the same identifier, so it is accepted.
        // Everything else (identifier and key) must be identical.
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
