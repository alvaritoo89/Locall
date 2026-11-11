# ADR 0025 · Phase 7a: detecting an engine that exited on its own

- **Status:** accepted
- **Date:** 2026-11-11

## Problem

The scheduler kept the loaded engine and, when the same model was requested again, lent it out **without checking that
its process was still alive**. If `llama-server` exited on its own (out of memory, a crash, someone killing it), every
message failed until the model was unloaded for inactivity (5 minutes by default), and the status kept saying
«Motor listo» (engine ready). `Motor::sigue_vivo` existed, but it needed `&mut` (`Child::try_wait`) and the scheduler
shares the engine between threads, so it was never used. Found by reading the code during the Phase 7 review.

## Decision

- `locall_system::proceso_vivo(&Child)`: asks Windows with `WaitForSingleObject(process, 0)` (a 0 ms wait: just "have
  you finished?"). It does not need `&mut`, so it works with the shared engine. `Motor::sigue_vivo` becomes `&self`.
- The `Arrancador` trait gains `sigue_vivo(&motor)` (always alive by default); `ArrancadorReal` implements it and, if
  the engine has died, logs the cause the engine left behind (`Motor::causa_del_cierre`).
- The scheduler:
  - **when lending** the model that is already loaded, checks that it is alive; if not, it removes it and loads it again
    (if someone has it on loan, it waits until they release it: their request will fail on its own);
  - **the watcher**, while a loaded model is unused, checks every 2 s that the engine is alive and, if not, removes it:
    the status becomes «Ningún modelo cargado» (no model loaded) instead of lying.

## Tests

- With an imitation engine that the test "kills": it is reloaded when requested, the watcher removes it on its own, and
  it is not taken away from whoever has it on loan.
- With the real engine: the test terminates the process it started itself; Locall removes it in under 2 s, loads a new
  one and answers.
