# ADR 0035 · Phase 8e: code signing with SignPath Foundation

- **Status:** accepted (application in progress)
- **Date:** 2026-11-13

## Context

Locall 0.1.0 is published without Authenticode code signing, so Windows SmartScreen warns when the installer is run.
The author chose SignPath Foundation's free code signing for open-source projects (Phase 8 decisions), which can only be
requested once a version has been released. Paid alternatives remain possible if the application is not accepted
(Azure Artifact Signing appears to be limited to individuals in the USA and Canada; a commercial certificate costs
money every year).

## What SignPath Foundation requires, and how Locall meets it

From its terms (signpath.org/terms) and its application form (signpath.org/apply):

| Requirement | Locall |
|---|---|
| OSI-approved license, no proprietary code | Apache-2.0; every distributed dependency has an accepted license (ADR 0031) |
| Already released in the form to be signed | 0.1.0, published on GitHub Releases (ADR 0033, 0034) |
| Built from source in a verifiable, automated way | GitHub Actions builds the installer from a tag on `main` (ADR 0033) |
| Only the project's own binaries are signed | `locall.exe`, `locall-app.exe` and the installer; llama.cpp is distributed unmodified (allowed inside a signed installer) |
| Roles: authors, reviewers, approvers; manual approval of every signing request | One maintainer holds every role; each release is already approved by hand |
| Multi-factor authentication for every team member, on SignPath and on the repository | The author must have it enabled on GitHub and on the SignPath account |
| A "code signing policy" on the home and download pages, naming the provider, the roles and a privacy policy | `firma.html`, linked from the footer of every page and from the download page |
| Software that sends data must say so and let the user turn it off | Locall sends no telemetry; the only connections (Hugging Face on request, the signed catalog at most once a day) are listed in the policy and the privacy page |
| Installers have a matching uninstaller; no system changes without warning | Tauri's NSIS uninstaller; the only system change is the user's `PATH`, undone on uninstall (ADR 0032) |
| Signed binaries carry the project's product name and version | Tauri sets them from `tauri.conf.json` and `Cargo.toml` |

The certificate is issued to **SignPath Foundation**, which appears as the publisher, not the author.

## Decision

1. **Now:** publish the code signing policy (`firma.html`, in Spanish with the required statements also in English,
   since SignPath reviews in English) and mention SignPath Foundation on the download page. The website tests allow
   links to `signpath.io` and `signpath.org`, and check that the policy is linked from every page.
2. **The author applies** through SignPath's form, with their own name and email, and accepts its code of conduct.
   Nothing is submitted on their behalf.
3. **If accepted:** integrate signing into the release workflow (SignPath's GitHub action pinned to a commit and an API
   token stored as a repository secret; the signing request waits for the author's approval in SignPath), and publish
   a signed version. That integration gets its own ADR.
4. **If rejected or postponed** (the project is new and the form asks for evidence of reputation): keep releasing with
   the minisign-signed hashes and apply again later, or reconsider a paid certificate.

## Consequences

- The website states plainly that signing is in progress, so users know why SmartScreen warns.
- Even signed, SmartScreen may keep warning for a while until the certificate and the file gain reputation.
