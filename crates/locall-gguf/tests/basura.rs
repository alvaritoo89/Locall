//! Malformed inputs against the GGUF reader (ADR 0026): valid headers deliberately broken. It must never crash nor
//! request more memory than is reasonable at once. (Models come from any Hugging Face author.)

#[global_allocator]
static MEDIDOR: locall_mutador::MedidorDeMemoria = locall_mutador::MedidorDeMemoria;

/// Builder for test GGUF headers.
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

/// A header with everything: texts, numbers of every type, lists (including lists of lists) and several tensors.
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

/// No metadata and no tensors.
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
