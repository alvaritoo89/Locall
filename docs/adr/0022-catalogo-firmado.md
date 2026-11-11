# ADR 0022 · Catalog of compatible models, signed with minisign

- **Status:** accepted
- **Date:** 2026-11-10

## Decision

- Locall has its **own catalog of compatible models** (`catalogo/catalogo.json` in this repository). It is the single
  source for the app, the CLI and the website.
- **Being in the catalog means "compatible":** the header of the exact file (pinned to its commit) was read, and the
  engine named in `checked_engine` recognizes its architecture and data types. **Models are neither downloaded nor
  tested** (the author's decision, 2026-11-10): that is why "tested" is never claimed anywhere (rule 19). A compatible
  format is a good sign, not a guarantee: a model with a very new architecture may fail to load with this engine
  version. No speeds are published. Each person sees in the app whether the model fits on **their** machine.
- Later (Phase 7 or 8) the models could be tested for real in GitHub Actions, with no downloads on the publisher's
  machine, and "compatible" could then be told apart from "tested".
- **Signed with minisign** (Ed25519). The private key is kept **only by the publisher**, on their machine and
  password-protected; it is never on GitHub or any server. The public key ships inside Locall
  (`locall_catalogo::CLAVE_PUBLICA`, id `5C09624CD1482FE0`).
- Verification uses **`minisign-verify` 0.3.0** (MIT, no dependencies, no `unsafe`; Tauri's official updater uses it
  too, so it will serve for Phase 8 updates). **Sigstore** was rejected: 44 direct dependencies (tokio, reqwest,
  openssl…) and a public internet log on every signature.
- **Signature first, content second:** an unsigned catalog is never interpreted. Only signatures in minisign's current
  format (with BLAKE2b) are accepted; the legacy format is rejected.
- **Against old catalogs:** `published` is part of what is signed. The app will never accept a catalog older than the
  last one it accepted (so nobody can serve back an old, correctly signed catalog containing a withdrawn model).
- **Strict validation** with limits (4 MB, 500 models, 16 variants, 64 files…): names, 40-character commits, 64-character
  sha256, relative `.gguf` paths without `..`. Unknown fields are ignored; a newer `format` asks the user to update
  Locall.
- `catalogo/**` is stored in Git **byte for byte** (`-text` in `.gitattributes`): if Git changed the line endings, the
  signature would no longer be valid.

## Format

See the documentation in `crates/locall-catalogo/src/lib.rs`. Field names in English, like the API: `format`,
`published` and `models` (`name`, `title`, `publisher`, `description`, `tags`, `license`, `context_length` and
`variants`: `name`, `default`, `repo`, `commit`, `checked_engine`, `files`: `path`, `size`, `sha256`).

`locall pull qwen3-4b` installs the default variant; `locall pull qwen3-4b:q8_0`, another one.

## How a new catalog is published

1. Edit `catalogo/catalogo.json` (bump `published`).
2. Sign it on the publisher's machine (PowerShell):

   ```powershell
   & "<path>\minisign.exe" -S -s "$env:USERPROFILE\.minisign\locall-catalogo.key" -m catalogo\catalogo.json
   ```

3. Push both files (`catalogo.json` and `catalogo.json.minisig`).

## Catalog v1 (2026-11-10)

12 models, all "compatible" with engine b12189 (header of the exact file read; architectures qwen3, gemma4, mistral3,
granite, llama, phi3 and gpt-oss) and none downloaded: Qwen3 1.7B, 4B and 8B; Gemma 4 E2B and E4B; Ministral 3 3B and
8B; Granite 4.2 3B and 8B; SmolLM2 1.7B; Phi-4; gpt-oss 20B. One variant per model (the Q4 quantization each author
publishes). All public, no account needed, licensed Apache-2.0 (Phi-4: MIT). Signed by the author with key
`5C09624CD1482FE0`. The Gemma 4 and Ministral 3 models also understand images; Locall uses them for text only.

Locall ships a copy (`locall_catalogo::incluido()`), verified exactly like a downloaded one; a test fails if someone
edits the catalog without signing it again.

## Consequences

- If the private key (or its password) is lost, no new catalogs can be signed until a version of Locall with another
  public key is released. That is why a backup must be kept.
- The tests use their own **test keys** (`crates/locall-catalogo/tests/datos/`); their private keys are not in the
  repository.
- The `minisign.exe` 0.12 used for signing was checked against its author's signature (key
  `RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3`, published in its README).
