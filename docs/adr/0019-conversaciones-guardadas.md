# ADR 0019 · Saved conversations: in the core, real deletion and one message at a time

- **Status:** accepted
- **Date:** 2026-11-09

## Decision

- **Conversations are stored by the core**, in the same SQLite registry as the models (schema v2: `conversaciones` and
  `mensajes` tables, with cascading deletes). The app and any other client see the same ones.
- **They are stored only on your computer** (`%LOCALAPPDATA%\Locall\locall.db`), never leave it (ADR 0008) and do not
  appear in the daily logs (ADR 0014).
- **Deleting really deletes.** When SQLite deletes, it does not overwrite the data: the text would remain in the file
  and in its write-ahead log (`-wal`). That is why the connection uses `PRAGMA secure_delete=ON` (it fills deleted data
  with zeros) and, when a conversation is deleted, the `-wal` is emptied (`wal_checkpoint(TRUNCATE)`). A test searches
  the files for the text before and after deleting.
- **Locall sends the model the whole saved conversation** (with the system instructions first): the client only sends
  the new message (`POST /api/conversations/{id}/messages`). The settings (model, instructions, temperature) are saved
  with the conversation on every message.
- **Nothing half-done:**
  - if the model fails (for example, the conversation no longer fits), the question is not kept;
  - if it is stopped before anything arrives, it is not kept either;
  - if it is stopped halfway, what arrived is saved, marked as cut off.
- **Each conversation handles one message at a time.** If another arrives while the previous one is still being saved
  (for example, stopping and sending again right away), it waits its turn, so messages are never saved out of order.
- **Before saving, Locall checks** that the conversation and the model exist (in that order).
- The app creates the conversation **when the first message is sent** (with its beginning as the title), so no empty
  conversations are left behind.

## Routes

| Route | What it does |
|------|----------|
| `GET /api/conversations` | List (most recent first) |
| `POST /api/conversations` | Create an empty one (`{"title"?}`) |
| `GET /api/conversations/{id}` | A conversation with its messages |
| `PATCH /api/conversations/{id}` | Change the title (`{"title"}`) |
| `DELETE /api/conversations/{id}` | Delete it for real |
| `POST /api/conversations/{id}/messages` | Write a message (`{"content", "model", "system"?, "temperature"?, "stream"?}`) and respond in NDJSON; the last event carries the saved conversation and message |

A conversation that does not exist gives `404 conversation_not_found`.

## Rationale

- **In the core rather than the app:** so the CLI can open the same conversations later, and the app stays just a UI
  (rule 9).
- **Real deletion:** local privacy is a design principle (rule 18). Someone who deletes a conversation expects it to be
  unrecoverable from disk.
- **The core assembles the whole conversation:** what is saved and what the model sees cannot differ.

## Consequences

- `secure_delete` makes deletion a little slower (it writes zeros). With text conversations it is not noticeable.
- Conversations are **not encrypted** on disk: they are protected by the Windows account, like the rest of your files.
  Encrypting them was decided in Phase 7 (Windows DPAPI).
- If you make backups that include `%LOCALAPPDATA%\Locall`, those copies may keep conversations that were already
  deleted. Locall cannot prevent that.
