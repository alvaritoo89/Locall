# ADR 0018 · Desktop app: Tauri 2, React, and the token kept out of JavaScript

- **Status:** accepted
- **Date:** 2026-11-09

## Decision

- The app (`crates/locall-app`) is built with **Tauri 2** (2.12, stable; version 3 is in *alpha*). It uses the WebView2
  that ships with Windows, so the app is small.
- The UI, in `crates/locall-app/ui`, is built with **React 19 + TypeScript + Vite**, with our own CSS (variables with
  the mock-ups' colors: black and white with a sage-green accent `#5B7464`, `#8FA597` in dark mode). No component
  libraries and no Tailwind.
- **The Rust side of the app is just another API client** (rule 9): it reuses `locall-cli`'s client and on-demand
  startup. The UI never talks to the API; it asks Rust for things through `invoke`.
  - **The token never reaches JavaScript.**
  - The API keeps rejecting requests with an `Origin` (ADR 0015): no exception needs to be opened for the app.
- **Strict content policy** (CSP): the UI only loads its own files (`'self'`) and only talks to Rust (`ipc:`).
- **Fonts ship inside the app** (Inter and JetBrains Mono from `@fontsource-variable`, OFL license): they are never
  requested from Google or anywhere else.
- The app looks for `locall.exe` next to itself (or in `LOCALL_EXE`) and starts it in the background if it is not
  running.
- The app is **not in the workspace's `default-members`**: `cargo build` and `cargo test` do not build it (Tauri is over
  200 crates and needs the built UI). For the app: `npm run tauri dev` in `crates/locall-app`.

## Rationale

- **Tauri rather than a native Rust UI** (egui, iced): the research mock-ups are HTML/CSS and are reused; it is easier
  to learn; Jan, a similar project, uses it successfully.
- **Tauri rather than Electron:** Electron bundles a whole browser (~100 MB per app).
- **The token in Rust:** if some malicious content ever managed to run in the UI (for example, a model's text rendered
  incorrectly), it could not steal the token or use the API on its own.

## Consequences

- **Dependencies:** Tauri builds 233 crates on Windows (almost all MIT/Apache; **5 are MPL-2.0**, which require keeping
  their license and crediting them when distributing). The UI installs 29 npm packages (93 MB in `node_modules`, outside
  Git; 0 known vulnerabilities at install time). All licenses are MIT, Apache-2.0 or OFL-1.1.
- Node.js is needed to build the UI (not to use the app).
- The current icon is **provisional** (an "L." drawn by us) until the final one exists.
- Testing the UI automatically is harder than testing the core: the Rust side is tested like everything else, and the
  screens are checked by looking at them.
