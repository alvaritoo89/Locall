# ADR 0020 · Explore, Settings and importing from the app

- **Status:** accepted
- **Date:** 2026-11-10

## Decision

### Explore: searching Hugging Face by name

- The core searches with `GET https://huggingface.co/api/models?search=…&filter=gguf&sort=downloads&direction=-1`
  (format checked live on 2026-11-10): only repositories with GGUF files, most downloaded first, at most 50.
- **Hugging Face only receives the search text** (encoded), with no account and no machine data. This **extends
  ADR 0008**: besides downloading, Locall queries Hugging Face when searching, and only when the person searches. The
  screen says so.
- The API offers it as `POST /api/hub/search` with `{"query", "limit"?}`: with the text in the body, what is searched
  never becomes part of a URL (nor appears in a request line), and no URL decoding is needed.
- Opening a result looks up the repository and installs it exactly as from the Library (version pinned to the commit,
  compatibility checked before downloading, sha256).
- Installs in progress are kept at the root of the UI (`App`), not in each screen: progress shows in Explore and in the
  Library and is not lost when switching screens.

### Settings

- `GET /api/settings` returns the settings **in use** (read when Locall started) and the **saved** ones (`config.toml`
  now), and whether a restart is needed. `PATCH /api/settings` saves changes.
- **Changes are applied when Locall restarts**, not live: the core reads its settings at startup, and changing them on
  the fly (the context of a loaded model, the scheduler's unload time…) would be fragile. The app offers «Reiniciar
  Locall ahora» (restart now: `POST /api/shutdown`; it starts again by itself), disabled while something is in
  progress.
- **Can be changed:** the context, the threads, the minutes before freeing memory and before shutting Locall down, and
  the days of logs kept. **Cannot be changed from the app** (only by hand in `config.toml`, with Locall closed):
  - the **models folder**: installed models would stay in the old one; moving them properly is separate work;
  - the **port**: the app and the CLI look for Locall on the port in the file; changing it while Locall is running would
    end with two instances running;
  - **paralelo**: fixed at 1 (ADR 0012).
- **Saving respects the user's file:** only the lines of the changed settings are rewritten (a comment at the end of
  that same line is lost; the others remain), the result is validated before writing, and it is written to a temporary
  file that then replaces the original.

### Importing a `.gguf` from the app

- The standard Windows file dialog is opened with **`rfd` 0.17.2** (MIT). It is the piece the official
  `tauri-plugin-dialog` uses internally, but without the plugin: the plugin pulls in `tauri-plugin-fs` and requires
  granting file permissions to the UI. On Windows, `rfd` only uses `windows-sys` and `raw-window-handle`, which were
  already in the project: it adds **one** crate. 0.17.2 was chosen over 0.18.0 because the latter was 3 days old.
- **The dialog is opened by the Rust side**, and the chosen path goes straight to the API: the UI never provides a
  path. The same applies to opening folders in File Explorer: the UI only picks «datos» (data) or «modelos» (models)
  and Locall provides the path.

## Consequences

- Search depends on Hugging Face's public API; if its format changes, Explore will fail with a clear error (there is a
  test against the real Hugging Face, marked `#[ignore]` because it uses the internet).
- Until our own catalog exists (Phase 6), Explore shows what is on Hugging Face without filtering for quality.
- Restarting unloads the model from memory: the next answer takes a few seconds longer to start.
