# ADR 0009 · Model registry in SQLite, with migrations

- **Status:** accepted
- **Date:** 2026-11-08

## Decision

- The registry of installed models lives in a **SQLite** database (`locall.db`), used through `rusqlite` with SQLite
  **bundled** into the binary (`bundled`, SQLite 3.54.1).
- **WAL** mode, foreign keys enabled, and a wait of up to 5 s if another process is writing.
- The schema is versioned (`PRAGMA user_version`). **Migrations** are applied in order, each in its own transaction.
  **A published migration is never modified**: changes go in a new one.
- If the database belongs to a **newer** version of Locall, it is left untouched and the user is asked to update.
- `STRICT` tables; dates as seconds since 1970 (UTC); on read, data is validated with `locall-store`'s types
  (`Huella`, `Segmento`, `NombreArchivo`).

## Rationale

- Transactions: registering or deleting a model is all or nothing.
- Several processes (core, CLI) can query it at the same time.
- Ollama had to maintain two storage formats at once because it had not versioned from the start.
- `bundled` avoids depending on the user having SQLite installed.

## Rejected alternatives

- **JSON files:** no transactions; risk of corruption on a power cut.
- **Other embedded databases (sled, redb…):** less well known, and without SQLite's tools for inspecting the data.

## Consequences

- Dependencies: `rusqlite` and `libsqlite3-sys` (MIT), SQLite (public domain); at build time, `cc` and similar
  (MIT/Apache, not shipped).
- The first build needs the Microsoft Build Tools (it compiles SQLite's C code).
