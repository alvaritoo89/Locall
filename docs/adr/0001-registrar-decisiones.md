# ADR 0001 · Record architecture decisions

- **Status:** accepted (language amended by ADR 0027)
- **Date:** 2026-11-08

## Context

Locall is a personal project with no deadlines that may be paused for months. The important decisions were made with
data (Phase 0), and we need to be able to remember **what** was decided and **why**.

## Decision

Every important technical decision is documented in a short ADR in `docs/adr/`, numbered (`NNNN-title.md`), with
these parts: status, date, context, decision, rationale, rejected alternatives and consequences. ADRs were originally
written in Spanish; since ADR 0027 they are written in English.

An ADR is never deleted: if a decision changes, a new one is written that **supersedes** it, and the old one becomes
"superseded by NNNN".

## Rationale

- Project rule 10: document important technical decisions.
- It makes it easier to pick the project up again and to explain decisions to future contributors.

## Consequences

- A bit more work when deciding; much less when remembering.
- The Phase 0 measurements are in the Phase 0 report (project documentation, outside the repository); the ADRs quote
  the key figures.
