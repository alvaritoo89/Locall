# ADR 0023 · The catalog inside Locall: updating, installing by name and recommending

- **Status:** accepted
- **Date:** 2026-11-10

## Decision

- **On startup**, the core uses the newest catalog (`published`) between the one it ships with and one downloaded
  earlier (`<data>\catalogo\`), always with the signature checked. If the saved one fails the signature check, the
  bundled one is used.
- **Updating** (`POST /api/catalog/update`): GitHub is checked
  (`https://raw.githubusercontent.com/alvaritoo89/Locall/main/catalogo/catalogo.json` and its `.minisig`) **at most
  once a day** (unless `force`). The time of the last check is saved even if it fails, so as not to insist when there
  is no internet.
  - Limits: 4 MB for the catalog, 4 KB for the signature.
  - It is only accepted if the signature is valid **and** `published` is greater than the current one's: an old, validly
    signed catalog is ignored (anti-rollback).
  - It is saved through a temporary file + rename; if interrupted halfway, the signature will not match on startup and
    the bundled catalog is used.
  - Nothing is checked automatically: the app requests the update the first time Explore is opened; the CLI, with
    `locall catalog`.
- **Installing by name** (`locall pull qwen3-4b`, `{"name": …}` in `POST /api/models/pull`): **before downloading**,
  Locall checks that what Hugging Face offers at that commit matches exactly what is signed (same parts, same sizes,
  same sha256). If not: a `catalog_mismatch` error and nothing is downloaded. Then the normal install (checks the
  header and verifies each file with sha256).
- **Identifier** of the installed model: the catalog name (`qwen3-4b`) for the default variant and `name-variant`
  (`qwen3-4b-q8_0`) for the others. So `locall run qwen3-4b` works after `locall pull qwen3-4b`.
- In the CLI, `pull` with a `/` is still a Hugging Face repository; without `/`, a catalog name.
- **Does it fit?** is **approximate** in the catalog: the model has not been downloaded and its header cannot be read,
  so it is estimated as the file + 10% + 512 MB, against the memory free right now (as in the Library). The app and the
  CLI say «aprox.» (approx.).
- The app shows «Recomendados por Locall» (recommended by Locall) at the top of Explore, labelled «compatible», with the
  explanation that they have not been tested one by one (ADR 0022).

## New routes and errors

| Route | What it does |
|------|----------|
| `GET /api/catalog` | Catalog in use: `published`, `source` (`builtin`/`downloaded`), `checked_at` and `models` (with `status: "compatible"`, `installed`, `fit_estimate`, `memory_estimate_bytes`, `size_bytes`, `variants`) |
| `POST /api/catalog/update` | `{"force"?}` → `{"result": "not_due" \| "no_changes" \| "updated", "catalog": …}` |

Errors: `not_in_catalog` (404), `catalog_mismatch` (409), `catalog_invalid` and `catalog_unavailable` (502).

## Privacy

Extension of ADR 0008: besides Hugging Face, Locall connects to **GitHub** (raw.githubusercontent.com), at most once a
day and only when Explore or `locall catalog` is used, to download two public files. Nothing but the request itself is
sent. The Settings screen says so.

## Tests

Test catalogs signed with the test key (`crates/locall-core/tests/datos/`), published after the official one, against a
fake Hugging Face that also serves the catalog: updating once a day, saving and remembering, anti-rollback, tampered
signature, GitHub without the file, installing by name, a different fingerprint on Hugging Face (nothing is
downloaded) and a non-existent name. Plus a test (`#[ignore]`, uses the internet) against the real catalog on GitHub.
