//! Names: validating Hugging Face repositories, recognizing split models and deducing the identifier, the display
//! name and the quantization from the file name.

/// Is `owner/name` a valid Hugging Face repository?
pub(crate) fn repo_valido(repo: &str) -> bool {
    let Some((dueno, nombre)) = repo.split_once('/') else {
        return false;
    };
    [dueno, nombre].iter().all(|parte| {
        !parte.is_empty()
            && parte.len() <= 96
            && !parte.starts_with('.')
            && parte
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    })
}

/// Is it a valid revision (branch, tag or commit)? `/` is allowed (for example `refs/pr/1`), but never `..`.
pub(crate) fn revision_valida(revision: &str) -> bool {
    !revision.is_empty()
        && revision.len() <= 128
        && !revision.contains("..")
        && !revision.starts_with('/')
        && revision
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
}

/// If `nombre` is a part of a split model (`base-00001-of-00003.gguf`), returns `(base, part, parts)`.
pub(crate) fn parte_de(nombre: &str) -> Option<(&str, u32, u32)> {
    let sin_extension = nombre.strip_suffix(".gguf")?;
    // "...-00001-of-00003": the last 15 characters.
    let (base, cola) = sin_extension.split_at_checked(sin_extension.len().checked_sub(15)?)?;
    let cola = cola.strip_prefix('-')?;
    let (parte, partes) = cola.split_once("-of-")?;
    if parte.len() != 5 || partes.len() != 5 {
        return None;
    }
    let (parte, partes): (u32, u32) = (parte.parse().ok()?, partes.parse().ok()?);
    (parte >= 1 && parte <= partes && partes >= 2).then_some((base, parte, partes))
}

/// Name of part `i` of `n` of a split model.
pub(crate) fn nombre_de_parte(base: &str, i: u32, n: u32) -> String {
    format!("{base}-{i:05}-of-{n:05}.gguf")
}

/// The file name without `.gguf` or the part suffix: `Qwen3-4B-Q4_K_M`.
pub(crate) fn nombre_para_mostrar(archivo: &str) -> String {
    if let Some((base, _, _)) = parte_de(archivo) {
        return base.to_owned();
    }
    archivo.strip_suffix(".gguf").unwrap_or(archivo).to_owned()
}

/// Short, valid identifier for the store: `qwen3-4b-q4_k_m`.
pub(crate) fn id_desde_archivo(archivo: &str) -> String {
    let mut id = String::new();
    for c in nombre_para_mostrar(archivo).chars() {
        let c = c.to_ascii_lowercase();
        let valido = c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | '-');
        let c = if valido { c } else { '-' };
        if !(c == '-' && id.ends_with('-')) {
            id.push(c);
        }
    }
    let id = id.trim_matches(|c| c == '-' || c == '.');
    let id: String = id.chars().take(64).collect();
    if id.is_empty() {
        "modelo".to_owned()
    } else {
        id
    }
}

/// Known quantizations, from longest to shortest (so `Q4_K_M` wins over `Q4_K`).
const CUANTIZACIONES: [&str; 34] = [
    "IQ2_XXS", "IQ3_XXS", "IQ2_XS", "IQ3_XS", "IQ4_XS", "IQ4_NL", "Q2_K_S", "Q3_K_S", "Q3_K_M",
    "Q3_K_L", "Q4_K_S", "Q4_K_M", "Q5_K_S", "Q5_K_M", "IQ1_S", "IQ1_M", "IQ2_S", "IQ2_M", "IQ3_S",
    "IQ3_M", "TQ1_0", "TQ2_0", "MXFP4", "Q2_K", "Q6_K", "Q4_0", "Q4_1", "Q5_0", "Q5_1", "Q8_0",
    "BF16", "F16", "F32", "Q3_K",
];

/// The quantization that appears in the file name (`Qwen3-4B-Q4_K_M.gguf` → `Q4_K_M`), if any.
pub(crate) fn cuantizacion(archivo: &str) -> Option<String> {
    let nombre = nombre_para_mostrar(archivo).to_ascii_uppercase();
    let mut candidatas: Vec<&str> = CUANTIZACIONES.to_vec();
    candidatas.sort_by_key(|c| std::cmp::Reverse(c.len()));
    for c in candidatas {
        for (i, _) in nombre.match_indices(c) {
            let antes = nombre[..i].chars().next_back();
            let despues = nombre[i + c.len()..].chars().next();
            let borde = |x: Option<char>| x.is_none_or(|x| !x.is_ascii_alphanumeric());
            if borde(antes) && borde(despues) {
                return Some(c.to_owned());
            }
        }
    }
    None
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn valida_repositorios_y_revisiones() {
        assert!(repo_valido("Qwen/Qwen3-4B-GGUF"));
        assert!(repo_valido("ggml-org/tiny-llamas"));
        for malo in [
            "", "qwen", "a/b/c", "/x", "x/", "../x", "a/.git", "a b/c", "a/c?x=1",
        ] {
            assert!(!repo_valido(malo), "{malo}");
        }
        assert!(revision_valida("main"));
        assert!(revision_valida("99dd1a73db5a37100bd4ae633f4cfce6560e1567"));
        assert!(revision_valida("refs/pr/1"));
        for mala in ["", "../main", "/main", "a b", "main?x"] {
            assert!(!revision_valida(mala), "{mala}");
        }
    }

    #[test]
    fn reconoce_las_partes() {
        assert_eq!(
            parte_de("qwen2.5-7b-instruct-q4_k_m-00002-of-00002.gguf"),
            Some(("qwen2.5-7b-instruct-q4_k_m", 2, 2))
        );
        assert_eq!(parte_de("modelo.gguf"), None);
        assert_eq!(
            parte_de("x-00003-of-00002.gguf"),
            None,
            "parte mayor que el total"
        );
        assert_eq!(
            parte_de("x-00001-of-00001.gguf"),
            None,
            "una sola parte no es dividido"
        );
        assert_eq!(parte_de("x-1-of-2.gguf"), None);
        assert_eq!(nombre_de_parte("x", 2, 3), "x-00002-of-00003.gguf");
    }

    #[test]
    fn deduce_identificador_y_nombre() {
        assert_eq!(id_desde_archivo("Qwen3-4B-Q4_K_M.gguf"), "qwen3-4b-q4_k_m");
        assert_eq!(
            id_desde_archivo("qwen2.5-7b-instruct-q4_k_m-00001-of-00002.gguf"),
            "qwen2.5-7b-instruct-q4_k_m"
        );
        assert_eq!(
            id_desde_archivo("Mi Modelo (versión 2).gguf"),
            "mi-modelo-versi-n-2"
        );
        assert_eq!(id_desde_archivo("???.gguf"), "modelo");
        assert_eq!(
            nombre_para_mostrar("Qwen3-4B-Q4_K_M.gguf"),
            "Qwen3-4B-Q4_K_M"
        );
        assert!(locall_store::Segmento::nuevo(&id_desde_archivo("Ñandú 3B.gguf")).is_ok());
    }

    #[test]
    fn reconoce_la_cuantizacion() {
        assert_eq!(
            cuantizacion("Qwen3-4B-Q4_K_M.gguf").as_deref(),
            Some("Q4_K_M")
        );
        assert_eq!(
            cuantizacion("stories15M-q4_0.gguf").as_deref(),
            Some("Q4_0")
        );
        assert_eq!(
            cuantizacion("qwen2.5-7b-instruct-q4_k_m-00001-of-00002.gguf").as_deref(),
            Some("Q4_K_M")
        );
        assert_eq!(cuantizacion("model-IQ4_XS.gguf").as_deref(), Some("IQ4_XS"));
        assert_eq!(cuantizacion("modelo-bf16.gguf").as_deref(), Some("BF16"));
        assert_eq!(cuantizacion("stories260K.gguf"), None);
    }
}
