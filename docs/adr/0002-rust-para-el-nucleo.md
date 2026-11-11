# ADR 0002 · Rust for the core

- **Status:** accepted
- **Date:** 2026-11-08

## Decision

Locall's core (model management, engine, local API and CLI, in a single binary) is written in **Rust**. The desktop
app will use Tauri 2, whose backend is also Rust.

## Rationale

- A single language for the core and the desktop shell.
- Memory safety and explicit error handling (`Result`), well suited to a process that runs for hours.
- Good access to the Windows API (Job Objects, demonstrated in Phase 1 with `windows-sys`).
- The author's learning goal: learning Rust and systems programming.

## Rejected alternatives

- **Go:** proven in production by Ollama and easier to learn; rejected because of the learning goal and to unify with
  Tauri. It remains a valid alternative if Rust ever became a bottleneck.

## Consequences

- A steep learning curve at first: offset by moving step by step, with explanations and tests.
- `unsafe` is limited to small system-integration modules, with every block justified.
