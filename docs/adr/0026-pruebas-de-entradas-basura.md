# ADR 0026 · Phase 7b: malformed-input testing (fuzzing) without dependencies

- **Status:** accepted
- **Date:** 2026-11-11

## Decision

- Everything Locall reads **from outside** is tested with thousands of **valid inputs corrupted on purpose**: flipping
  bits and bytes, putting huge numbers where sizes go, inserting, removing, repeating and cutting chunks. It must
  **always** end with a controlled result (usually an error), without:
  - crashing (`panic`);
  - allocating more than **16 MB** at once (measured by our own memory allocator, `MedidorDeMemoria`, installed only in
    the test programs).
- A crate of our own **for tests only** (`crates/locall-mutador`), **with no dependencies**: an xorshift64* generator
  with a fixed seed (everything is reproducible) and, if an input fails, it is saved to a file with its number and
  seed.
- `cargo-fuzz` (needs *nightly* Rust and does not work well on Windows) and `proptest`/`arbitrary` (dependencies for
  something solved in ~200 lines) were rejected.
- `cargo test` tries few inputs so it stays fast; `LOCALL_BASURA=<n>` runs long passes.

## What is tested

| What | Why it comes "from outside" |
|-----|------------------------|
| GGUF headers (`locall-gguf`) | Models come from any author on Hugging Face |
| The catalog, its signature and the key (`locall-catalogo`) | It comes from GitHub (or from someone impersonating it) |
| Settings (`locall-config`) | `config.toml` is edited by hand |
| Response chunks, errors and the engine log (`locall-engine`) | A malicious model could make the engine write strange things |
| Chat, conversation and OpenAI-compatible requests (`locall-api`) | Any program on the machine that has the token |

## What it found

- **A real bug, fixed:** the GGUF reader allocated the memory that a text's length **claimed** (up to 64 MB) before
  checking that the data existed. A file of a few KB caused a 33.5 MB allocation at once. Now it allocates as the data
  arrives (starting with 64 KB).
- **Checked, not a problem:** `minisign-verify` does not look at the key's algorithm marker (the first two letters) and
  ignores extra lines at the end of a signature. In both cases the cryptographic key and signature are identical; it
  never accepts something signed with another key.

Long passes (optimized build, 2026-11-11): 2 million GGUF headers, 600,000 catalogs/signatures/keys, 1 million settings
files, 1.5 million engine inputs and 1.5 million API requests. No failures.

## Also (review of `expect` calls and indexing)

Every `expect` and index in the code that was reviewed is protected by an earlier validation. One was improved: if the
app window cannot open (for example, WebView2 is missing), the release build no longer exits without a word: it shows
a Windows error dialog with the reason.
