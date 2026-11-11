# ADR 0011 · Understandable engine errors

- **Status:** accepted
- **Date:** 2026-11-08

## Decision

- `locall-engine` recognizes the most common `llama-server` failures and turns them into a `Causa` with **our own
  message in Spanish** (the UI language): port in use, out of memory, damaged or incomplete model, unsupported
  architecture, context exceeded, model loading and key rejected.
- **Sources:** for load failures, the end of the engine log (last 64 KB). For request errors, the `type` field of the
  error JSON, which is more stable than the message.
- **The engine's text is never shown** to the user: it may contain the model's path, including the Windows user name
  (D4). If the cause is unknown, the message says what failed and where the log is.
- The only exception to "classify only for messages" (D8): if the cause is **port in use**, `arrancar_listo` retries
  with another port (up to 3 times). If a future version changed that text, it would simply not retry and the error
  would be shown. There is no risk of doing something wrong.

## Rationale

- Phase 0 showed that the engine's messages are technical and in English ("failed to allocate buffer for kv cache").
- The texts we look for were checked by **triggering each failure** with build b12189. The unit tests keep those real
  lines, so an engine version change that breaks them will be noticed.
- The port is chosen free right before launching the engine, but another program can take it at that instant; with
  the retry, the user never sees it.

## Rejected alternatives

- **Showing the engine's original message:** incomprehensible to the user and containing private data.
- **Passing an already-open socket to the engine** (avoiding the port race): `llama-server` does not support it.

## Consequences

- When updating the llama.cpp build, the real-failure tests (`tests/motor_real.rs`, `tests/chat_real.rs`) must be
  rerun in case the texts change.
- Not verified yet: Vulkan-specific failures (GPU memory). They are classified as unknown until we trigger and
  recognize them.
