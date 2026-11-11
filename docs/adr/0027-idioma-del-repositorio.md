# ADR 0027 · Repository language: documentation and comments in English

- **Status:** accepted (amends the language part of ADR 0001)
- **Date:** 2026-11-11

## Decision

- **In English:** the README, the ADRs, crate descriptions, configuration comments, GitHub workflows, commit messages
  and **every comment in the code** (Rust, TypeScript, CSS, JavaScript).
- **Stays in Spanish:**
  - **what users see:** the app, the command line, error messages and the website. Locall's product language is
    Spanish; other languages may come later (Phase 9);
  - **identifiers** in the code (functions, types, variables, crate and file names such as `locall-catalogo`,
    `leer_peticion`, `Nucleo`): renaming them would mean tens of thousands of changed lines with a real risk of breaking
    things, for no functional gain. The README explains this.
- Existing ADRs were translated keeping their content, figures and dates; their file names stay the same because the
  README and the code link to them.

## Rationale

- English documentation makes the project understandable to the wider open-source community (contributors,
  reviewers, people evaluating it).
- Translating only comments is safe: a check strips the comments from every file before and after translating and
  requires the remaining code to be **identical**.

## Consequences

- Mixed languages in the code (Spanish names, English comments). Acceptable and documented.
- New comments and documentation are written in English from now on; user-facing text stays in Spanish.
