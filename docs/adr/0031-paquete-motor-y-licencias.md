# ADR 0031 · Phase 8a: what goes into the package, the engine and third-party licenses

- **Status:** accepted
- **Date:** 2026-11-12

## Context

Phase 8 publishes Locall as a Windows installer (decisions by the author: a per-user NSIS `setup.exe` without
administrator rights; version 0.1.0 unsigned, then free signing from SignPath Foundation; only the CPU engine, since
the GPU stays disabled until it is validated, D1). Before the installer, the contents had to be defined: where each
piece lives once installed, which engine files are shipped and how they are verified, and how the licenses of
everything distributed are honoured.

## Decision

**Layout once installed** (it already worked this way; now it is the contract):

```text
<install folder>\
├── locall-app.exe              the desktop app; starts locall.exe next to it
├── locall.exe                  the core and the command line
├── motor\                      the engine: llama-server.exe and its libraries (+ their licenses)
├── LICENSE                     Locall's license (Apache-2.0)
└── THIRD-PARTY-NOTICES.txt     everything else that is distributed, with its licenses
```

User data stays where it always was (`%LOCALAPPDATA%\Locall`), separate from the program.

**The engine** (`scripts/preparar_motor.ps1`): the official llama.cpp **b12189** CPU build, downloaded from the
project's GitHub releases (or taken from a local copy) and accepted **only** if its sha256 is exactly the pinned one
(the same one recorded in Phase 0). From the zip, only `llama-server.exe` and the libraries it loads are kept (24 files,
about 45 MB): the other programs (benchmarks, quantizers, other command-line tools) and the RPC backend for remote
computers are left out. llama.cpp's MIT license is not in the zip, so it is downloaded from the repository at the same
tag and also checked by its sha256. Finally the script runs `llama-server --version` to confirm it starts and is the
expected build. The engine binaries are not signed by llama.cpp; we distribute them unmodified (SignPath allows unsigned
upstream libraries inside a signed installer).

**Third-party notices and license check** (`scripts/avisos_de_terceros.py`): it walks the dependencies that actually
end up in the programs (normal dependencies of `locall` and `locall-app` for Windows, and the non-development npm
packages of the UI) and writes `THIRD-PARTY-NOTICES.txt` with each component's name, version, license and license
texts. With `--comprobar` it only checks that every license is in the accepted list: permissive licenses (MIT,
Apache-2.0, BSD, ISC, Zlib, Unicode, Unlicense, CC0, BSL…), **MPL-2.0** (distributed unmodified, with a link to each
component's source, as that license asks) and **OFL-1.1** (the fonts). Anything else, including undeclared licenses,
fails. The CI runs this check on every change, so a new dependency with an unexpected license cannot reach a release.

At the time of writing: 274 Rust crates and 6 npm packages, all accepted. Five MPL-2.0 crates come in through Tauri
(`cssparser`, `cssparser-macros`, `dtoa-short`, `option-ext`, `selectors`).

No new dependency: the scripts use only PowerShell and Python's standard library, both present on Windows CI runners.

## Tests

- The script refuses a zip with a different sha256 (tried with the Vulkan zip) and creates nothing.
- The real-engine tests (load, chat, model switch, crash detection, idle unload: 91 tests) pass with `LOCALL_MOTOR`
  pointing at the prepared folder, so the reduced set of files is enough.
- The license checker accepts and rejects the right expressions (`OR`, `AND`, `WITH`, parentheses, the old `/` form,
  GPL/AGPL/LGPL, undeclared).
