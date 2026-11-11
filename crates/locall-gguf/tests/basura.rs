//! Entradas basura contra el lector GGUF (ADR 0026): cabeceras válidas estropeadas a propósito. Nunca debe cerrarse de
//! golpe ni pedir de una vez más memoria de la razonable. (Los modelos vienen de cualquier autor de Hugging Face.)

#[global_allocator]
static MEDIDOR: locall_mutador::MedidorDeMemoria = locall_mutador::MedidorDeMemoria;

/// Constructor de cabeceras GGUF de prueba.
#[derive(Default)]
struct C(Vec<u8>);

impl C {
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u64(&mut self, v: u64) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn texto(&mut self, t: &str) -> &mut Self {
        self.u64(t.len() as u64);
        self.0.extend_from_slice(t.as_bytes());
        self
    }
}

/// Una cabecera con de todo: textos, números de todos los tipos, listas (también de listas) y varios tensores.
fn completa() -> Vec<u8> {
    let mut c = C::default();
    c.0.extend_from_slice(b"GGUF");
    c.u32(3).u64(3).u64(9);
    c.texto("general.architecture").u32(8).texto("llama");
    c.texto("general.name").u32(8).texto("Modelo de prueba ñ");
    c.texto("llama.block_count").u32(4).u32(16);
    c.texto("llama.context_length").u32(10).u64(4096);
    c.texto("llama.rope.freq_base")
        .u32(6)
        .0
        .extend_from_slice(&10000f32.to_le_bytes());
    c.texto("general.flag").u32(7).0.push(1);
    c.texto("tokenizer.ggml.tokens")
        .u32(9)
        .u32(8)
        .u64(3)
        .texto("a")
        .texto("bb")
        .texto("ccc");
    c.texto("tokenizer.ggml.scores")
        .u32(9)
        .u32(6)
        .u64(2)
        .u32(0)
        .u32(0);
    c.texto("anidada")
        .u32(9)
        .u32(9)
        .u64(1)
        .u32(4)
        .u64(2)
        .u32(1)
        .u32(2);
    for (i, tipo) in [0u32, 1, 12].into_iter().enumerate() {
        c.texto(&format!("blk.{i}.weight"))
            .u32(2)
            .u64(32)
            .u64(4)
            .u32(tipo)
            .u64(0);
    }
    c.0
}

/// Sin metadatos ni tensores.
fn minima() -> Vec<u8> {
    let mut c = C::default();
    c.0.extend_from_slice(b"GGUF");
    c.u32(3).u64(0).u64(0);
    c.0
}

#[test]
fn las_bases_se_leen() {
    assert!(locall_gguf::leer(&completa()[..]).is_ok());
    assert!(locall_gguf::leer(&minima()[..]).is_ok());
}

#[test]
fn el_lector_gguf_aguanta_entradas_basura() {
    locall_mutador::probar(
        "gguf",
        0x6767_7566,
        20_000,
        &[completa(), minima()],
        |entrada| {
            let _ = locall_gguf::leer(entrada);
        },
    );
}
