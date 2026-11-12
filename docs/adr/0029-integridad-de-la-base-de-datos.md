# ADR 0029 · Phase 7d: database integrity check and copy before migrating

- **Status:** accepted
- **Date:** 2026-11-12

## Problem

`locall.db` (installed models and saved conversations) was opened without checking it. A file damaged by a disk failure,
a power cut at a bad moment or a manual edit made Locall fail with SQLite errors at random points, or not start at all.
And migrations (ADR 0009) changed the user's only copy: a migration with a bug could leave the data wrong with no way
back. The next one (Phase 7e, encrypting the conversations) rewrites every message, so this matters now.

## Decision

**On every open** (`Registro::abrir`), SQLite's quick check (`PRAGMA quick_check`) runs; for a database of this size it
takes milliseconds. If the file is damaged (the check fails, or SQLite says it is corrupt or not a database):

- It is **set aside, never deleted**: renamed to `locall.db.danada-<seconds>`, together with its `-wal` and `-shm`. The
  user can still try to recover something from it.
- A new, empty database is created and Locall keeps working.
- The core adds a warning (shown on the app's Home screen and by `locall serve`) saying where the old file is, that the
  previous conversations are in it, and that downloaded models are still in the store and can be installed again
  without downloading them.

Other errors (for example, the file being locked by another process) are **not** treated as damage: they are reported as
they are, and nothing is renamed.

The user chose this over refusing to start: Locall stays usable and nothing is lost, because the damaged file is kept.

**Before migrating** an existing database (version > 0 and lower than the current one):

- A consistent copy is made next to it with `VACUUM INTO`: `locall.db.copia-antes-de-v<N>`. It includes what is still
  in the `-wal`.
- After migrating, the full check runs (`integrity_check` and `foreign_key_check`).
- If everything went well, the copy is deleted. If anything failed, the copy is kept and the error says where it is.
- A copy left by an earlier failed attempt is never overwritten: it is the older, safer one.

A new database (version 0) has nothing to copy.

## Privacy note

The copy contains the conversations as they are in the database. It is deleted with a normal file deletion, which on
an SSD does not guarantee that the data is unrecoverable (the same applies to any file). It only exists for the
duration of a migration. Phase 7e must take it into account: the copy made before encrypting holds the unencrypted
messages.

## Tests

- A file that is not a database, and a real database with a page overwritten with garbage: both are set aside intact and
  a new one is used. A healthy database is not set aside.
- A v1 database is upgraded to v2 and the copy disappears afterwards.
- A migration that fails: the database stays at the previous version and the copy is a complete, valid database with
  the data from before. A copy from an earlier attempt is not overwritten. A new database makes no copy.
- In the core: the warning appears and says where the damaged file is.
