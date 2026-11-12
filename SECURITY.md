# Security policy

Locall runs AI models on your own computer and is built so that your data never leaves it. If you find a way to break
that promise, or any other security problem, thank you for reporting it.

## Reporting a vulnerability

**Please do not open a public issue.** Use GitHub's private reporting instead:

1. Go to the repository's **Security** tab.
2. Click **Report a vulnerability**.
3. Describe the problem, how to reproduce it and what an attacker could achieve.

Only the maintainer can see the report. You will get an answer as soon as possible; Locall is maintained by one person,
so please allow a few days. Once a fix is published, you will be credited in the release notes if you wish.

## Supported versions

Locall has not had a stable release yet (see the [status](README.md#status)). Only the latest code on the `main` branch
is supported: fixes are not backported.

## What is in scope

Anything that breaks Locall's security or privacy design, for example:

- the local API (`127.0.0.1`) being reachable or usable without its token, from another computer or from a website
  ([ADR 0015](docs/adr/0015-nucleo-y-api-local.md));
- a malicious GGUF model file, catalog or server response that crashes Locall, makes it use unbounded memory, writes
  outside its folders or runs code ([ADR 0026](docs/adr/0026-pruebas-de-entradas-basura.md));
- the engine process escaping its limits ([ADR 0006](docs/adr/0006-seguridad-del-motor.md),
  [ADR 0028](docs/adr/0028-limites-del-motor.md));
- the model catalog being accepted without a valid signature, or an older catalog replacing a newer one
  ([ADR 0022](docs/adr/0022-catalogo-firmado.md));
- conversations, questions, answers or keys ending up in logs, in plain text on disk, or sent anywhere
  ([ADR 0008](docs/adr/0008-privacidad-sin-telemetria.md), [ADR 0014](docs/adr/0014-registros-privados.md),
  [ADR 0030](docs/adr/0030-conversaciones-cifradas.md)).

## What is out of scope

- Malware already running as your Windows user: it can do anything you can, including asking Windows to decrypt your
  conversations ([ADR 0030](docs/adr/0030-conversaciones-cifradas.md) explains why).
- Vulnerabilities in llama.cpp itself: please report them to the [llama.cpp project](https://github.com/ggml-org/llama.cpp).
  If Locall makes one easier to exploit, that is in scope.
- What a model says. Models can be wrong or produce harmful text; Locall shows their answers as plain text and never
  runs them as code or HTML.
