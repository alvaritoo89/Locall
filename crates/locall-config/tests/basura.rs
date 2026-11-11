//! Malformed inputs against the settings (ADR 0026): anyone can edit `config.toml` by hand; a malformed file must
//! give a clear error, never crash Locall.

use locall_config::lector::Valor;
use locall_config::{ARCHIVO_POR_DEFECTO, Config, texto_con_cambios};

#[global_allocator]
static MEDIDOR: locall_mutador::MedidorDeMemoria = locall_mutador::MedidorDeMemoria;

#[test]
fn los_ajustes_aguantan_entradas_basura() {
    let bases = vec![
        ARCHIVO_POR_DEFECTO.as_bytes().to_vec(),
        br#"contexto = 8_192 # comentario
hilos = 'auto'
carpeta_modelos = 'D:\Modelos'
"puerto" = 1"#
            .to_vec(),
        "\u{feff}contexto = 4096\nhilos = \"\u{00f1}\t\"\n"
            .as_bytes()
            .to_vec(),
    ];
    locall_mutador::probar("config", 0x000C_0F16, 20_000, &bases, |entrada| {
        let texto = String::from_utf8_lossy(entrada);
        let _ = Config::desde_texto(&texto);
        let _ = texto_con_cambios(
            &texto,
            &[
                ("contexto", Valor::Entero(8192)),
                ("hilos", Valor::Texto("auto".into())),
            ],
        );
    });
}
