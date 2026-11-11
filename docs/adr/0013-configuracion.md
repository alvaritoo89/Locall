# ADR 0013 · Configuration: data folder and config.toml with our own reader

- **Status:** accepted
- **Date:** 2026-11-08

## Decision

- Locall's data lives in **`%LOCALAPPDATA%\Locall`**: `config.toml`, `locall.db`, `modelos\` and `registros\`. The
  `LOCALL_DATOS` environment variable selects another folder (tests, portable version).
- The v1 settings: `carpeta_modelos`, `hilos` (`"auto"` or a number), `contexto`, `inactividad_minutos`, `paralelo` and
  `registros_dias`. Each one with a **validated range** and error messages that give the line.
- The file uses a **subset of TOML** (`key = value`, `#` comments, `'...'` and `"..."` strings, integers) read with
  **our own reader** (`locall-config::lector`).
- If the file does not exist, the defaults are used. `crear_si_no_existe` creates it with the defaults, commented, and
  **never overwrites** the user's file.
- An unknown key is a **warning**, not an error.

## Rationale

- `Local` rather than `Roaming`: the "Roaming" part can be synchronized to other machines, and models take gigabytes.
- **No new dependencies** (rule 3): with the `toml` + `serde` crates it would be about ten dependencies to read six flat
  settings. Our reader is about 150 lines with tests. Since the format is valid TOML, it can switch to `toml` in the
  future **without touching users' files**.
- Windows paths are written with single quotes (`'D:\Modelos'`) because in TOML double quotes interpret `\`. If someone
  uses double quotes, the message explains it.
- Warning instead of failing on unknown keys lets an older version open a file written by a newer one.
- Threads are validated against the machine's: in Phase 0, 246 threads on a 12-thread machine sank the speed.

## Rejected alternatives

- **`toml` + `serde`:** many dependencies for such a simple file.
- **JSON (we already have `serde_json`):** no comments, and easy to break by hand with a comma.
- **The Windows registry:** hard to edit, to copy and to port to Linux.

## Consequences

- If sections or lists are ever needed, switch to the `toml` crate (same format).
- The GPU (Vulkan) setting will be added once it is validated (D1).
