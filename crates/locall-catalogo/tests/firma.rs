//! La firma del catálogo, con archivos firmados de verdad por minisign 0.12 con **claves de prueba** (sus claves
//! privadas no están en el repositorio y no sirven para nada más).

use locall_catalogo::{CLAVE_PUBLICA, ErrorCatalogo, verificar};

const CATALOGO: &[u8] = include_bytes!("datos/catalogo.json");
const FIRMA: &str = include_str!("datos/catalogo.json.minisig");
const FIRMA_OTRA_CLAVE: &str = include_str!("datos/firmado-por-otra.minisig");
const FIRMA_FORMATO_ANTIGUO: &str = include_str!("datos/formato-antiguo.minisig");

/// La clave pública de prueba (la segunda línea de `prueba.pub`).
fn clave(archivo: &str) -> String {
    archivo.lines().nth(1).unwrap().trim().to_owned()
}

fn clave_de_prueba() -> String {
    clave(include_str!("datos/prueba.pub"))
}

#[test]
fn un_catalogo_bien_firmado_se_acepta() {
    let c = verificar(CATALOGO, FIRMA, &clave_de_prueba()).unwrap();
    assert_eq!(c.publicado, 1_794_300_000);
    assert_eq!(c.modelos.len(), 2);
}

#[test]
fn un_byte_cambiado_invalida_la_firma() {
    let mut cambiado = CATALOGO.to_vec();
    // Cambiar el tamaño de un archivo (por ejemplo, para colar otro): 1000000 → 1000001.
    let pos = cambiado.windows(7).position(|w| w == b"1000000").unwrap();
    cambiado[pos + 6] = b'1';
    assert_eq!(
        verificar(&cambiado, FIRMA, &clave_de_prueba()),
        Err(ErrorCatalogo::FirmaIncorrecta)
    );
}

#[test]
fn los_saltos_de_linea_de_windows_tambien_invalidan_la_firma() {
    // Por eso .gitattributes guarda estos archivos byte a byte.
    let crlf = String::from_utf8(CATALOGO.to_vec())
        .unwrap()
        .replace('\n', "\r\n");
    assert_eq!(
        verificar(crlf.as_bytes(), FIRMA, &clave_de_prueba()),
        Err(ErrorCatalogo::FirmaIncorrecta)
    );
}

#[test]
fn firmado_con_otra_clave_no_se_acepta() {
    assert!(verificar(CATALOGO, FIRMA_OTRA_CLAVE, &clave_de_prueba()).is_err());
    // Y con la clave que sí lo firmó, sí: la firma es buena, pero no es de quien esperamos.
    assert!(
        verificar(
            CATALOGO,
            FIRMA_OTRA_CLAVE,
            &clave(include_str!("datos/otra.pub"))
        )
        .is_ok()
    );
}

#[test]
fn la_clave_oficial_no_acepta_el_catalogo_de_prueba() {
    assert!(verificar(CATALOGO, FIRMA, CLAVE_PUBLICA).is_err());
}

#[test]
fn el_formato_antiguo_de_firma_se_rechaza() {
    assert_eq!(
        verificar(CATALOGO, FIRMA_FORMATO_ANTIGUO, &clave_de_prueba()),
        Err(ErrorCatalogo::FirmaIncorrecta)
    );
}

#[test]
fn una_firma_o_una_clave_mal_escritas_se_explican() {
    assert_eq!(
        verificar(CATALOGO, "esto no es una firma", &clave_de_prueba()),
        Err(ErrorCatalogo::FirmaIlegible)
    );
    assert_eq!(
        verificar(CATALOGO, FIRMA, "esto no es una clave"),
        Err(ErrorCatalogo::ClaveNoValida)
    );
}

#[test]
fn un_archivo_enorme_se_rechaza_sin_leerlo() {
    let enorme = vec![b' '; locall_catalogo::TAMANO_MAX + 1];
    assert!(matches!(
        verificar(&enorme, FIRMA, &clave_de_prueba()),
        Err(ErrorCatalogo::DemasiadoGrande(_))
    ));
}

#[test]
fn la_clave_oficial_es_una_clave_valida() {
    // Si alguien estropea la constante, la app no podría verificar nunca el catálogo oficial.
    let error = verificar(CATALOGO, FIRMA, CLAVE_PUBLICA).unwrap_err();
    assert_ne!(error, ErrorCatalogo::ClaveNoValida);
}
