# ADR 0030 · Phase 7e: conversations encrypted with the user's Windows key (DPAPI)

- **Status:** accepted
- **Date:** 2026-11-12

## Problem

Saved conversations (ADR 0019) were stored in plain text in `locall.db`. Anyone who got hold of that file could read
them: a backup, a sync service such as OneDrive, another Windows user of the same computer, or someone with the disk in
their hands. Conversations are the most private thing Locall keeps.

## Decision

The conversation texts are encrypted with **Windows DPAPI** (`CryptProtectData` / `CryptUnprotectData`, user scope),
the option the author chose at the start of Phase 7:

- The key is derived by Windows from the current user's credentials. Locall never sees, generates or stores a key, and
  there is nothing for the user to remember.
- A fixed "purpose" (`Locall: conversaciones v1`) is passed as DPAPI's extra entropy: decrypting requires it.
- No new dependency: only the `Win32_Security_Cryptography` feature of `windows-sys`, already in the project. The calls
  live in `locall-system` (`proteger` / `desproteger`), the only crate with `unsafe` for Windows.

**What is encrypted:** the conversation title, the system instructions, and each message's text and reasoning.
**What is not:** dates, token counts, speed, whether an answer was cut, the model name and the temperature. They reveal
nothing about what was said, and they are needed to sort and list conversations without decrypting everything. An empty
text is stored empty (an empty reasoning or system prompt has nothing to hide).

**Schema version 3:** the `STRICT` tables only accepted text in those columns, so the migration (the first one written
in Rust, not SQL) renames the old tables, creates new ones with `BLOB` columns, encrypts every existing row into them and
drops the old ones, all in one transaction. The 7d mechanism (ADR 0029) applies: a copy before migrating, a full check
afterwards, the copy deleted on success. After any migration of an existing database, the file is now rebuilt
(`VACUUM`) and the change log emptied, so the old plain-text pages do not stay inside the file.

**If a text cannot be decrypted**, it is shown as a fixed notice («No se puede descifrar este texto…») instead of
failing: one unreadable message does not make the conversation or the list unusable.

## What it protects and what it does not

- **Protects against:** someone who gets the `locall.db` file without the user's Windows session: a backup, OneDrive,
  another Windows user on the same computer, a disk removed from the computer.
- **Does not protect against:** malware running as the same user with the session open. Like Locall, it can ask
  Windows to decrypt. No program can prevent that.
- **Risk of loss:** if an administrator **force-resets** the user's Windows password, or Windows is reinstalled, the
  DPAPI key is lost and the saved conversations can no longer be read (they show the notice above). A normal password
  change by the user keeps everything readable.
- **Leftovers on disk:** Locall zeroes deleted data (`secure_delete`), rebuilds the file after migrating, and deletes the
  pre-migration copy (which holds the plain text for the duration of the migration). Even so, an SSD may keep old
  copies of blocks where no program can reach them. Full protection against that is disk encryption (BitLocker).

## Tests

- `proteger` / `desproteger`: round trip (including emoji and empty text), the text does not appear inside the result,
  two encryptions of the same text differ, and decryption fails with another purpose, with modified bytes or with
  garbage.
- Registry: everything written is read back intact, and the secret text does not appear in the database files, not even
  after closing. A message that cannot be decrypted shows the notice while the rest of the conversation works.
- Migration: a v2 database with plain-text conversations is upgraded to v3 keeping ids, dates, figures, models and
  texts, and afterwards the plain text is no longer anywhere in the files and the copy is gone.
- Real deletion (ADR 0019) is still checked: the encrypted bytes of a deleted message disappear from the files.
