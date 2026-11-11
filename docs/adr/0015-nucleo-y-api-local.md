# ADR 0015 · Core and local API (Phase 3)

- **Status:** accepted
- **Date:** 2026-11-08

## Decision

### Organization (rule 9)

- **`locall-core`**: the core, with no HTTP. It brings the store, registry, downloads, scheduler and configuration
  together in a `Nucleo` shared between threads.
- **`locall-api`**: only the HTTP server, security and translation to JSON.
- **`locall`**: the program (`locall serve` in this phase; the other commands in Phase 4). A single executable for the
  core and the CLI (decision confirmed during the research phase).

### Installing from Hugging Face

1. The requested revision (`main`, a tag…) is resolved to its **exact commit**, and that commit is used for everything
   else.
2. The file list at that commit gives the size and the **sha256** (`lfs.oid`) of each GGUF. A file without a sha256 is
   **not downloaded**: it could not be verified.
3. Before downloading, **only the header** of the GGUF is read over the network and compatibility is checked
   (`locall-compat`): a model the engine cannot run is not downloaded.
4. Split models are installed with all their parts, which keep their original names.
5. Gated repositories are rejected with a clear message: v1 uses no Hugging Face account or token.

### Importing from disk

A GGUF already on disk is added with a **hard link** if it is on the same disk as the store (it does not take double
the space) or with a copy otherwise. When the model is deleted from Locall, the user's original file is not touched.

### Deleting

The registry entry, the model's folder and the store files that **no other model** uses are deleted. If the model is
loaded and nobody is using it, it is unloaded from memory; if it is in use, it is not deleted.

### API (parts 3b–3e)

- Server with **axum + tokio**. The core uses plain threads and is called with `spawn_blocking`.
- Only on **127.0.0.1**, with a **token** stored in `%LOCALAPPDATA%\Locall\token`. Requests with a `Host` other than
  `localhost` or `127.0.0.1` are rejected (against "DNS rebinding"), as are those coming from websites that are not
  allowed (`Origin`).
- **Routes and fields in English** (an API for programmers; the OpenAI-compatible part requires it); messages for
  people in Spanish, with a fixed code for programs.

## Rationale

- **axum** rather than tiny_http (no releases since October 2022) or bare hyper: it is maintained, the most widely used,
  and its HTTP parser has been reviewed by far more people than anything of ours (security, rule 1). It brings 28 new
  dependencies (all MIT/Apache, plus one MIT+BSD-3), but tokio will arrive anyway with the desktop app (Tauri).
- Pinning the commit and requiring sha256 makes installs **reproducible and verifiable**.
- Reading only the header avoids downloading useless gigabytes.

## Consequences

- `verificado = false` for everything installed until the verified catalog exists.
- Third-party licenses that require attribution (BSD-3) must appear in a notices file when distributing.
- If a model is deleted right as another thread starts loading it, the load fails with a clear error. It is a very
  small window and corrupts nothing.
