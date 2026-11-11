//! Example: does this model fit on my machine RIGHT NOW?
//!
//! Usage:  cargo run -p locall-compat --example cabe -- <file.gguf> [context]
//! For a split model, give the first part: all the parts next to it are added up.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(ruta) = args.next() else {
        eprintln!("Uso: cargo run -p locall-compat --example cabe -- <archivo.gguf> [contexto]");
        return ExitCode::FAILURE;
    };
    let contexto: u64 = args.next().and_then(|c| c.parse().ok()).unwrap_or(4_096);

    let cabecera = match locall_gguf::leer_archivo(&ruta) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("No se pudo leer {ruta}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let bytes = tamano_de_todas_las_partes(Path::new(&ruta), cabecera.resumen().partes);
    let memoria = match locall_system::memoria() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("No se pudo consultar la memoria: {e}");
            return ExitCode::FAILURE;
        }
    };

    let ev = locall_compat::evaluar(&cabecera, bytes, contexto, memoria.disponible);
    let gb = |b: u64| b as f64 / 1e9;
    println!("Modelo:            {ruta}");
    println!("Contexto:          {contexto} tokens");
    println!("Pesos:             {:.2} GB", gb(ev.estimacion.pesos));
    if let Some(kv) = ev.estimacion.cache_kv {
        println!("Caché KV:          {:.2} GB", gb(kv));
    }
    println!("Memoria estimada:  {:.2} GB", gb(ev.estimacion.total));
    println!("RAM disponible:    {:.2} GB", gb(memoria.disponible));
    println!("Veredicto:         {}", ev.encaje.descripcion());
    if ev.comprobacion.es_compatible() {
        println!("Compatibilidad:    compatible con el motor (build 12189)");
    }
    for p in &ev.comprobacion.problemas {
        println!("Problema:          {p}");
    }
    for a in &ev.comprobacion.avisos {
        println!("Aviso:             {a}");
    }
    ExitCode::SUCCESS
}

/// Adds up the size of the `-0000N-of-0000M.gguf` parts found next to the given file.
fn tamano_de_todas_las_partes(primera: &Path, partes: Option<u64>) -> u64 {
    let tamano = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    let (Some(total), Some(nombre)) = (partes, primera.file_name().and_then(|n| n.to_str())) else {
        return tamano(primera);
    };
    let sufijo = format!("-00001-of-{total:05}.gguf");
    let Some(base) = nombre.strip_suffix(&sufijo) else {
        return tamano(primera);
    };
    (1..=total)
        .map(|i| {
            let parte: PathBuf =
                primera.with_file_name(format!("{base}-{i:05}-of-{total:05}.gguf"));
            tamano(&parte)
        })
        .sum()
}
