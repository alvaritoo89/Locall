# ADR 0034 · Phase 8d: the download page and the first public version

- **Status:** accepted
- **Date:** 2026-11-13

## Context

Version 0.1.0 was published on 2026-11-13 following ADR 0033: built by GitHub Actions, `SHA256SUMS.txt` signed by the
author with the project's minisign key, published by hand. Checked afterwards from outside, as any user would: the
three files download, the signature is valid («Locall 0.1.0») and the installer's hash matches.

## Decision

**The website links to the latest *published* version, set by hand.** `locall_web::VERSION_PUBLICADA` holds it, and
the download page builds the installer's link from it
(`github.com/alvaritoo89/Locall/releases/download/v<version>/Locall_<version>_x64-setup.exe`). It is deliberately
separate from the project's version in `Cargo.toml`: that one goes up when a version is being prepared, long before it
is published, and the website must never link to an installer that does not exist yet. The order for every release
is: publish on GitHub, then change `VERSION_PUBLICADA` and push (which republishes the website). A test checks that the
release notes of `VERSION_PUBLICADA` exist.

**The download page** (`descargar.html`) shows the download button with the version and size, how to install, an
honest explanation of the SmartScreen warning (the installer is not code-signed yet, why, and how to continue), where
the user's data is kept and that uninstalling does not delete it, the requirements, and how to check the installer
with minisign and the project's public key (the same one as the catalog; a test checks they are identical). The home
page now announces the version instead of «en desarrollo», and the privacy page states that Locall does not look for
updates on its own.

**The README** explains how to install from the release, how to check it, and how to run a source build (with the
engine prepared by `scripts/preparar_motor.ps1`).

## Consequences

- Locall can be downloaded and installed by anyone from the website: this is the public launch of the first version.
- Until Phase 8e (code signing with SignPath Foundation), Windows shows the SmartScreen warning; the website, the
  README and the release notes say so plainly.
