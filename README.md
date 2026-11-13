<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/logo-oscuro.png">
    <img src="docs/assets/logo-claro.png" alt="Locall." width="300">
  </picture>
</p>

<p align="center"><b>AI models on your own computer.</b><br>Free, local and private.</p>

<p align="center"><a href="https://alvaritoo89.github.io/Locall/">Website</a> · <a href="https://alvaritoo89.github.io/Locall/modelos/index.html">Models</a></p>

<p align="center">
  <img src="docs/assets/insignia-estado.svg" alt="status: in development">
  <img src="docs/assets/insignia-licencia.svg" alt="license: Apache-2.0">
  <img src="docs/assets/insignia-rust.svg" alt="rust: 1.99+">
  <img src="docs/assets/insignia-plataforma.svg" alt="platform: Windows">
  <img src="docs/assets/insignia-privacidad.svg" alt="privacy: no telemetry">
</p>

---

**Locall** is a free, open-source platform to download, manage and run artificial intelligence models **on your own
machine**. No accounts, no subscriptions, and your conversations never leave your computer.

> [!NOTE]
> Locall is **in development**. It can already be used (desktop app and command line) by building it from source (see
> [Try it](#try-it)); a signed installer is planned for Phase 8.
>
> Locall's user interface (the app, the command line and the website) is in **Spanish**. The code, its comments and
> the documentation are in English.

## Principles

| | |
|---|---|
| **Local first** | Models run on your computer. Privacy is a design principle, not an option. |
| **Truly free** | No mandatory paid services and no servers to pay for per user. |
| **Honest** | Never claim compatibility or speed without having measured it. |
| **For any computer** | Designed for laptops without a dedicated graphics card too. |

## How it works

```
   CLI  ·  Desktop app  ·  Your applications
                    │  local API (only on your computer)
                    ▼
         Locall core (Rust)
         models · verified downloads · memory
                    │  separate, protected process
                    ▼
          llama.cpp (official binaries)
```

A Rust core manages the models and drives the [llama.cpp](https://github.com/ggml-org/llama.cpp) inference engine as
a separate process. The command line, the desktop app and the local API are all clients of that core.

## Try it

There is no installer yet. In the meantime, from source (Windows 11; Windows 10 has not been verified yet):

1. Install [Rust](https://www.rust-lang.org/) 1.99 or newer and Visual Studio Build Tools (C++).
2. Build Locall:

   ```bash
   cargo build --release
   ```

3. Download the engine: `llama-b12189-bin-win-cpu-x64.zip` from
   [llama.cpp release b12189](https://github.com/ggml-org/llama.cpp/releases/tag/b12189)
   (sha256 `f912ea5e3e397c63405c81e732131ab0fa6e72411c80546bb5ad643f7dd82aac`). Extract it into
   `target\release\motor\`, so that you end up with `target\release\motor\llama-server.exe`.
4. Use it from `target\release` (in PowerShell, programs in the current folder are run with a leading `.\`):

   ```text
   .\locall catalog                                         recommended (compatible) models
   .\locall pull qwen3-4b                                   install one from the catalog
   .\locall pull Qwen/Qwen3-4B-GGUF                         which versions does it have?
   .\locall pull Qwen/Qwen3-4B-GGUF Qwen3-4B-Q4_K_M.gguf    install it (2.5 GB)
   .\locall run qwen3-4b-q4_k_m                             chat (Ctrl+C stops the answer)
   .\locall list                                            your models
   .\locall help                                            all commands
   ```

Locall starts by itself in the background when needed and exits on its own after a while without use. Its data
(models, settings, saved conversations, logs) lives in `%LOCALAPPDATA%\Locall` and never leaves your computer. Other
programs can use it through the [local API](docs/adr/0015-nucleo-y-api-local.md), also in an OpenAI-compatible format
([ADR 0016](docs/adr/0016-compatibilidad-openai-y-cierre.md)).

## Measured, not promised

Real tests on an ordinary laptop (recent laptop processor, integrated graphics and 16 GB of RAM). Numbers will differ
on other computers:

| Model | Generation (CPU) | Prompt processing (integrated GPU) | Memory (no mmap) |
|---|---|---|---|
| Qwen2.5 1.5B · Q4_K_M | ~33 tokens/s | ~509 tokens/s | ~1.2 GB |
| Qwen3 4B · Q4_K_M | ~12 tokens/s | ~189 tokens/s | ~2.5 GB |
| Qwen2.5 7B · Q4_K_M | ~7.6 tokens/s | ~115 tokens/s | ~4.6 GB |

## Status

- [x] Phase 0: research and proofs of concept on real hardware
- [x] Phase 1: engine control from Rust (startup, streaming, shutdown, protection against abrupt exits)
- [x] Phase 2: core (GGUF reader, model store, verified downloads, engine, scheduler, configuration and private logs)
- [x] Phase 3: local API (core, token-protected server, chat, downloads, OpenAI-compatible format, idle shutdown)
- [x] Phase 4: command line (`locall pull`, `run`, `list`, `show`, `rm`, `import`, `status`, `stop`) · **MVP**
- [x] Phase 5: desktop app (home, library, chat with saved conversations and real deletion, Hugging Face explorer, settings, import, keyboard shortcuts, accessibility, dark mode)
- [x] Phase 6: signed catalog of compatible models, and the website ([alvaritoo89.github.io/Locall](https://alvaritoo89.github.io/Locall/))
- [x] Phase 7: hardening (recovery when the engine dies, malformed-input testing, engine process limits, database integrity checks, encrypted conversations, Windows CI and a security policy)
- [ ] Phase 8: installer, code signing and distribution · in progress: package contents, engine and licenses ✓, Windows installer ✓, release workflow ✓

## Repository layout

| Path | Contents |
|---|---|
| [`crates/locall-gguf`](crates/locall-gguf) | GGUF header reader with safety limits |
| [`crates/locall-store`](crates/locall-store) | Model store: sha256 fingerprints, hard-link snapshots, disk space and cleanup |
| [`crates/locall-system`](crates/locall-system) | Machine information (memory, CPU, threads), Windows Job Objects with process limits, processes, clipboard and DPAPI encryption |
| [`crates/locall-compat`](crates/locall-compat) | Does this model fit, and can the engine run it? Memory and compatibility estimates |
| [`crates/locall-mutador`](crates/locall-mutador) | Tests only: malformed-input testing (fuzzing) and a memory meter |
| [`crates/locall-registry`](crates/locall-registry) | Registry of installed models and saved conversations (SQLite with integrity checks, migrations, encrypted conversations and real deletion) |
| [`crates/locall-download`](crates/locall-download) | sha256-verified, resumable downloads with stall detection |
| [`crates/locall-engine`](crates/locall-engine) | Starts, watches and stops `llama-server` safely, and streams chats with it (with cancellation) |
| [`crates/locall-scheduler`](crates/locall-scheduler) | Scheduler: one model at a time, memory check before loading, first-come-first-served queue, idle unload and dead-engine recovery |
| [`crates/locall-config`](crates/locall-config) | Data folder, user settings (`config.toml`) and daily logs without conversation data |
| [`crates/locall-catalogo`](crates/locall-catalogo) | Catalog of compatible models: format, validation and minisign signature |
| [`crates/locall-core`](crates/locall-core) | Core: install from Hugging Face (pinned to a commit and verified), import, list, delete, chat and save conversations |
| [`crates/locall-api`](crates/locall-api) | Local API (axum): 127.0.0.1 only, token, and protection against websites (`Host`, `Origin`) |
| [`crates/locall-cli`](crates/locall-cli) | Command line: an API client that starts Locall in the background when needed |
| [`crates/locall-app`](crates/locall-app) | Desktop app (Tauri 2 + React): just another API client, with the token kept out of the UI |
| [`crates/locall-web`](crates/locall-web) | Generates the website (static pages) from the signed catalog; published automatically to GitHub Pages |
| [`crates/locall`](crates/locall) | The `locall` program: `locall serve` and the command-line commands |
| [`catalogo`](catalogo) | The signed catalog of compatible models |
| [`web`](web) | Website styles, fonts, logos and script |
| [`docs/adr`](docs/adr) | Architecture decision records and their rationale |
| [`scripts`](scripts) | Packaging (engine download and verification, third-party notices and license check) and an OpenAI-client test |
| [`.github/workflows`](.github/workflows) | Continuous integration on Windows and website publishing |

## Development

Requirements: those in [Try it](#try-it). Some tests use the real engine and models and skip themselves if they are not
found (default paths under `C:\Locall-poc`); tests that download from the internet are marked `#[ignore]`.

```bash
cargo test
```

The desktop app (`crates/locall-app`) also needs [Node.js](https://nodejs.org/); run `npm install` and
`npm run tauri dev` inside that folder to start it in development mode.

To build the Windows installer (`target\release\bundle\nsis\Locall_<version>_x64-setup.exe`), from PowerShell at the
repository root ([ADR 0032](docs/adr/0032-instalador.md)):

```powershell
powershell -ExecutionPolicy Bypass -File scripts\construir_instalador.ps1
```

A note on naming: identifiers in the code (functions, types, variables) are in Spanish, the project's original
language; comments and documentation are in English.

## Security

Found a security problem? Please report it privately, as explained in [SECURITY.md](SECURITY.md).

## License

[Apache-2.0](LICENSE). AI models have their own licenses: Locall shows them when you look up a repository
(`locall pull <repository>`) and in each model's details (`locall show`).
