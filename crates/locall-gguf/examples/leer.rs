//! Ejemplo: muestra la "etiqueta" de un modelo GGUF.
//!
//! Uso:  cargo run -p locall-gguf --example leer -- <ruta del archivo .gguf>

use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(ruta) = std::env::args().nth(1) else {
        eprintln!("Uso: cargo run -p locall-gguf --example leer -- <archivo.gguf>");
        return ExitCode::FAILURE;
    };

    let cabecera = match locall_gguf::leer_archivo(&ruta) {
        Ok(c) => c,
        Err(error) => {
            eprintln!("No se pudo leer {ruta}: {error}");
            return ExitCode::FAILURE;
        }
    };

    let r = cabecera.resumen();
    let texto = |v: Option<u64>| v.map_or_else(|| "?".to_string(), |n| n.to_string());

    println!("Archivo:          {ruta}");
    println!(
        "Arquitectura:     {}",
        r.arquitectura.as_deref().unwrap_or("?")
    );
    println!("Capas:            {}", texto(r.capas));
    println!("Contexto máximo:  {} tokens", texto(r.contexto_max));
    println!(
        "Cabezas / KV:     {} / {}",
        texto(r.cabezas),
        texto(r.cabezas_kv)
    );
    println!("Vocabulario:      {} tokens", texto(r.vocabulario));
    println!(
        "Plantilla chat:   {}",
        if r.plantilla_chat { "sí" } else { "no" }
    );
    if let Some(partes) = r.partes {
        println!("Partes:           {partes} archivos");
    }
    println!(
        "Cabecera:         {:.2} MB",
        cabecera.bytes_cabecera as f64 / 1e6
    );
    for contexto in [4_096u64, 8_192, 32_768] {
        if let Some(bytes) = cabecera.cache_kv_bytes(contexto, 2) {
            println!(
                "Caché KV a {contexto:>6}: {:.0} MiB",
                bytes as f64 / (1024.0 * 1024.0)
            );
        }
    }
    ExitCode::SUCCESS
}
