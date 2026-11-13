# ADR 0033 · Phase 8c: how a version is released

- **Status:** accepted
- **Date:** 2026-11-13

## Decision

Releases are prepared by GitHub Actions and **published by the author**, by hand.

**Version numbers.** One version for the whole project, in three places that must agree: `[workspace.package]` in
`Cargo.toml` (every crate inherits it), `crates/locall-app/tauri.conf.json` (the installer's name and the version
Windows shows) and `crates/locall-app/package.json` (with its lock file). The first public version is **0.1.0**.

**Release notes** live in the repository, in `docs/releases/<version>.md`, written for users: in Spanish like the app
and the website, with one line in English at the top. Like everything public, they say only what has been tested and
include no information about the author's computer.

**The workflow** (`.github/workflows/release.yml`) runs only when a tag `vX.Y.Z` is pushed. Before building it checks,
and stops otherwise:

- the tag has the form `vX.Y.Z` and matches the version in the three files;
- the tagged commit is on `main`, so nothing can be released from another branch;
- the notes `docs/releases/X.Y.Z.md` exist;
- there is no release for that tag yet: an existing one is never overwritten.

It then builds the installer with the same script as locally (`scripts/construir_instalador.ps1`, ADR 0031 and 0032:
the engine is only accepted with its pinned sha256 and every third-party license must be accepted), writes
`SHA256SUMS.txt` (the `sha256sum` format, UTF-8 with LF line endings so the signed bytes are the same everywhere) and
creates a **draft** release with the installer and `SHA256SUMS.txt`. A draft is only visible to the repository's
owner. Only this job can write to the repository, and only to create the release; the only third-party action is the
official checkout, pinned to a commit; the GitHub CLI that comes with the runner creates the draft.

**Signing and publishing, by the author:**

1. Download `SHA256SUMS.txt` from the draft and check that the installer's hash is the expected one if wanted.
2. Sign it with minisign and the project key, on their own computer (the private key and its password never leave
   it):

   ```powershell
   minisign -Sm SHA256SUMS.txt -s "$env:USERPROFILE\.minisign\locall-catalogo.key" -t "Locall 0.1.0"
   ```

3. Upload `SHA256SUMS.txt.minisig` to the draft, review the notes and press **Publish**.

The release is signed with **the same minisign key as the model catalog** (ADR 0022, key id `5C09624CD1482FE0`): one
project key that users can learn once, and the author already keeps it safe. A catalog signature cannot be passed off
as a release signature, because each one covers the exact bytes of its own file. Users verify with the public key that
is in the notes, the website and the app.

Signing the program itself (Authenticode, so Windows stops warning) is a separate step: Phase 8e, with SignPath
Foundation, once this first version is published.

## Consequences

- Publishing a version is two commands and a few clicks for the author, and nothing becomes public without them.
- A mistake in the tag, the version numbers or the notes stops the workflow before anything is built.
- If a draft is wrong, the author deletes the draft and the tag, fixes the problem and tags again.
