# ADR 0012 · One conversation at a time per model, and a first-come-first-served queue

- **Status:** accepted
- **Date:** 2026-11-08

## Decision

- The engine is started with **`--parallel 1`** (`ConfigMotor::paralelo`, default `PARALELO_POR_DEFECTO = 1`). It
  serves one conversation at a time; if two arrive together, the second waits inside the engine and completes
  correctly (checked with b12189).
- The scheduler serves requests **in order of arrival** (turns):
  - a request for the model that is already loaded **does not jump ahead** of one waiting for another model;
  - consecutive requests for the same model go through together (a single load);
  - a request can be **cancelled** while it waits for its turn (`usar_con`); the next one goes through right away.
- `paralelo` is configurable: later (piece 10) it can be raised on powerful machines or for API clients that make
  several requests at once.

## Rationale

- By default, b12189 opens **4 slots** that **share** the context (`kv_unified`): with 4 long conversations at once,
  each would in practice get a quarter of the context, and the user would not understand why.
- On a laptop CPU, generating 4 answers at once makes **each one slower**. For the main use (one person chatting), a
  single conversation at full speed is better.
- With one slot, the engine reuses the previous conversation's cache better.
- Memory is the same with 1 or 4 slots (the context is a single shared block), so `locall-compat`'s estimate remains
  valid.
- Without first-come-first-served ordering, a request for another model could wait forever if requests for the loaded
  model kept arriving.

## Rejected alternatives

- **4 slots (the engine's default):** a split context and slower answers without warning.
- **Giving each slot its own context** (`--kv-unified-per-slot`): multiplies memory by the number of slots.

## Consequences

- Two simultaneous requests to the same model are answered **one after the other**. The API (Phase 3) must report that
  the second one is waiting.
- Cancellation in the queue is checked every 50 ms (the canceller only flips an `AtomicBool`).
- A load that has already started is not interrupted on cancel: the engine cannot be stopped halfway safely.
