# ADR 0003 · Official llama.cpp as a subprocess, with our own scheduler

- **Status:** accepted
- **Date:** 2026-11-08

## Decision

- The inference engine is **llama.cpp's `llama-server`, unmodified official binaries**, with a pinned build verified
  by sha256 (today: build 12189).
- It runs as a **child process**, never linked into the core, inside a Windows **Job Object** with
  `KILL_ON_JOB_CLOSE`.
- Locall uses **its own scheduler** (v1: one model loaded at a time) and **not** llama-server's router mode.

## Rationale

- Ollama ended up running the original `llama-server` as a subprocess; Jan, even though it builds it into its program,
  still runs it in a separate process, because `GGML_ASSERT` terminates the process and cannot be caught.
- Phase 1: without a Job Object, killing the core left an orphaned `llama-server` holding 1.21 GB; with a Job Object,
  Windows terminated it by itself. 10 consecutive cycles with no orphans and no memory leaks.
- Router mode is outside llama.cpp's security scope (`SECURITY.md`) and left orphaned processes behind for Jan after
  updating.

## Rejected alternatives

- **Linking llama.cpp as a library (FFI):** an engine crash would take the core down, and we would have to build the
  backends (CUDA, Vulkan) ourselves.
- **Building our own engine (like Jan):** too much complexity to start with; the architecture allows reconsidering it.
- **llama-server router mode:** see the rationale.

## Consequences

- We depend on llama-server's HTTP contract and command-line options: every build change is validated with regression
  tests before it is shipped.
- The official binaries are not signed and their ZIP does not include llama.cpp's MIT license: Locall will add the
  license notices.
