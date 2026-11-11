# ADR 0006 · Engine security: key, network and responses (D3, D4)

- **Status:** accepted
- **Date:** 2026-11-08

## Decision

When it launches `llama-server`, Locall:

1. Makes it listen **only on `127.0.0.1`**, on a **new port** chosen at every start.
2. Generates a **random 256-bit key** at every start and passes it through the **`LLAMA_API_KEY`** environment
   variable, never as a command-line argument.
3. Restricts CORS with `--cors-origins localhost` and disables the web UI (`--no-webui`).
4. **Never exposes the engine's port or forwards its responses as-is:** the core translates them into its own format,
   with the model's name instead of its path.

## Rationale (contract verified in Phase 0)

- Without the key, `/v1/models`, `/props`, `/slots` and `/metrics` return 401; `/health` answers without the key.
- Default CORS: it echoes any `Origin` with `Access-Control-Allow-Credentials: true`.
- `/v1/models` and every streaming chunk include the model's **full path** (which contains the Windows user name).
- Jan passes the key through an environment variable because any program on the machine can read a process's
  arguments.
- After a forced stop, the port stays in an orphaned "Listen" state for a few seconds.

## Consequences

- Phase 1 verified that the key does not appear in the engine's log.
- Locall's public API has its own authentication (a local token), independent of this key.
