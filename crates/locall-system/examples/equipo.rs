//! Example: what Locall knows about your computer.
//!
//! Usage:  cargo run -p locall-system --example equipo

fn main() {
    let p = locall_system::procesador();
    println!(
        "Procesador:        {}",
        p.nombre.as_deref().unwrap_or("(desconocido)")
    );
    println!("Hilos lógicos:     {}", p.hilos_logicos);
    println!("Hilos para motor:  {}", locall_system::hilos_para_motor());

    match locall_system::memoria() {
        Ok(m) => {
            println!("RAM total:         {:.1} GB", m.total as f64 / 1e9);
            println!("RAM disponible:    {:.1} GB", m.disponible as f64 / 1e9);
        }
        Err(e) => println!("RAM:               no disponible ({e})"),
    }

    match locall_system::memoria_de_proceso(std::process::id()) {
        Ok(b) => println!("Este programa usa: {:.1} MB", b as f64 / (1024.0 * 1024.0)),
        Err(e) => println!("Memoria propia:    no disponible ({e})"),
    }
}
