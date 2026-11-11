# ADR 0014 · Logs: what Locall does, never what you say

- **Status:** accepted
- **Date:** 2026-11-08

## Decision

- Every Locall crate writes through the standard **`log`** facade (`log::info!`, `log::warn!`). Our own writer
  (`locall-config::registros`) stores it in **`registros\locall-YYYY-MM-DD.log`**, one file per day, using Windows
  local time (`GetLocalTime`).
- Only messages from **our crates** (`locall…`) at **INFO level or higher** are stored. Messages from other crates
  (`ureq`…) and debug messages are not.
- On startup, `locall-*.log` files older than `registros_dias` days (7 by default) are deleted. No other file in the
  folder is touched.
- Writing to the log **never makes Locall fail**: if the disk fails, the line is lost.
- **Privacy (rule 17):** the log records what Locall **does** (model loaded or unloaded and why, engine started or
  stopped, downloads, retries, errors, timings and token counts). It **never** records conversation content
  (questions, answers, reasoning), keys or access tokens.

## Rationale

- `log` was already in the project (`ureq` brings it in) and is the Rust standard: **no new dependency**. Our writer is
  about 100 lines; `env_logger` or `tracing` would bring quite a few more dependencies.
- A log that contains conversations is a risk: users share it when asking for help. Hence the rule "numbers only", and
  a **test with the real engine** (`locall-scheduler/tests/registros_real.rs`) asks a question containing a secret word
  and checks that neither the question, nor the answer, nor the engine key appears.
- Filtering by crate avoids depending on what other crates decide to log.

## Rejected alternatives

- **`env_logger`, `tracing` or `flexi_logger`:** more dependencies for the same result.
- **Logging to the Windows Event Viewer:** hard for the user to read and to share.
- **UTC timestamps:** easier to compute, but confusing to read ("why does it say 19:00 when it was 20:00?").

## Consequences

- Anyone adding a `log::info!` must respect the privacy rule; the test covers the main path (load, chat, unload the
  model), not every future line.
- Error messages that include the path of the engine log contain the user's folder. That is acceptable because the file
  is local, but it must be kept in mind if "send logs" is ever offered.
- The engine's own log (`llama-server`) is separate: it is overwritten on every start and the cleanup does not touch
  it.
