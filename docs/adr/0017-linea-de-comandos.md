# ADR 0017 · Command line: an API client, on-demand startup and lexopt

- **Status:** accepted
- **Date:** 2026-11-09

## Decision

- The CLI (`locall-cli`) is a **client of the local API**, like the desktop app: it never touches the core directly
  (rule 9). It reads the port from `config.toml` and the token from `%LOCALAPPDATA%\Locall\token`.
- **On-demand startup** (confirmed decision): if Locall is not running, the commands that need it launch
  `locall serve` in the background and wait for it to respond (up to 20 s; if it exits earlier, that is reported along
  with the log). `status` and `stop` never start it.
- The background core is launched:
  - **without a console** (`DETACHED_PROCESS`) and in **its own process group** (`CREATE_NEW_PROCESS_GROUP`): a Ctrl+C
    in the terminal does not reach it;
  - **outside the terminal's Job Object** if the terminal allows it (`CREATE_BREAKAWAY_FROM_JOB`), so it does not die
    when the terminal is closed; if it is not allowed, it is launched anyway;
  - **without inheriting the console's input and output** (see Rationale).
- `locall stop` uses `POST /api/shutdown`, which shuts the core down just like Ctrl+C: in an orderly way, stopping the
  engine.
- Commands are parsed with **lexopt** (MIT, no dependencies). The help and every error message are our own, in
  Spanish. Exit codes: 0 = success, 1 = error, 2 = badly written command.

## Rationale

- A single path (the API) for every client: the API's security and tests also cover the CLI.
- **Inherited output:** on Windows, a child process inherits the parent console's "pipes". In the first version, the
  core inherited the CLI's and the terminal waited for it to close: `locall list` took **90 s** to finish (until the
  core shut down from inactivity). Marking the CLI's input and output as non-inheritable (`SetHandleInformation`)
  brings it down to **0.12 s**.
- **Short probe:** on Windows, connecting to a closed `localhost` port takes ~2 s because the system retries. To find
  out whether Locall is running, a 300 ms connection timeout is used; if the server exists, the system accepts the
  connection immediately even if the program is busy. `status` with Locall closed goes from 2 s to 0.3 s.
- **lexopt rather than clap:** clap is the most widely used, but its error messages are in English and changing them
  all is complicated. Minimal clap brings 4 dependencies; with `derive`, 21. lexopt brings none: it only solves the
  hard parts (`--option=value`, `-abc`, non-Unicode arguments).

## Consequences

- If another program holds the configured port, Locall does not start and the CLI says so, with the log path.
- If the core shuts down from inactivity between two commands, the next one starts it again (noticeable for a moment
  the first time).
- The help text (`ordenes::AYUDA`) must be maintained by hand when commands are added.
