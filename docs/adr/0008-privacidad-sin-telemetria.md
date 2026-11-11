# ADR 0008 · Local privacy: no telemetry

- **Status:** accepted (extended by ADR 0020 and ADR 0023)
- **Date:** 2026-11-08

## Decision

- Locall includes **no telemetry or analytics** of any kind.
- The only data that leaves the machine is:
  - **model downloads** from Hugging Face;
  - **catalog and update checks** from GitHub (can be turned off).
- Conversations, prompts and user files are **never** sent.
- The app will show a **connection log**: what left the machine, when and why.
- External APIs (OpenAI, Anthropic…) will only exist as an optional future feature, with each user's keys stored in
  the Windows Credential Manager.

## Rationale

- Project rules 17 and 18: no invasive analytics; local privacy is a design principle.
- Differentiation from tools that enable telemetry by default or rely on cloud services.
- Without our own servers, the project stays free.

## Consequences

- Problems are diagnosed with a **diagnostic bundle** that the user reviews and decides whether to send.
- There will be no usage metrics: priorities are decided through issues and explicit feedback.
