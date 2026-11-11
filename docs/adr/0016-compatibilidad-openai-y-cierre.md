# ADR 0016 · OpenAI compatibility and idle shutdown

- **Status:** accepted
- **Date:** 2026-11-08

## Decision

### OpenAI API compatibility

- `GET /v1/models` and `POST /v1/chat/completions`, in OpenAI's format: a whole `chat.completion` or, with
  `"stream": true`, `chat.completion.chunk` pieces over **SSE** ending with `data: [DONE]`. Supports
  `stream_options.include_usage`, `max_tokens` / `max_completion_tokens`, `temperature`, `seed`, the `developer` role
  and content given as a list of text parts.
- The "API key" is **Locall's token** (OpenAI SDKs already send `Authorization: Bearer …`). The same protections as
  everywhere else apply (ADR 0015): `Host` and `Origin`.
- What is **not supported yet is rejected** with a clear 400 error instead of being ignored: `tools`/`functions`,
  `tool_choice`, `response_format`, `n` > 1, images in messages and the `tool` role.
- Sampling parameters that Locall does not pass to the engine yet (`top_p`, `presence_penalty`, `frequency_penalty`,
  `stop`…) **are ignored**.
- Errors keep our `{"error": {"code", "message"}}` format, which includes the `message` that OpenAI clients read.

### Idle shutdown

- Setting `cierre_inactividad_minutos` (30 by default, 0 = never): if Locall has spent that long **with no requests
  and no work in progress**, it shuts itself down, stopping the engine. The CLI and the app start it again when needed
  (confirmed decision: on-demand core, not started with Windows).
- Work in progress means an install, a load or an answer under way, even if no new requests arrive. A loaded but unused
  model does not count. Checked every 30 s.
- `locall serve` ends with Ctrl+C or by idle shutdown, whichever comes first.

## Rationale

- Being OpenAI-compatible makes many existing tools and libraries work unchanged.
- Rejecting what is not supported prevents a tool from believing, for example, that it received a function call when
  Locall ignored it (rule 12: do not hide errors).
- A core that shuts itself down does not linger in the background using memory when the user no longer needs it.

## Verification and limits (rule 19)

- **Checked** with tests that follow OpenAI's documented format, against the real engine: whole answers, SSE with usage
  at the end, and errors.
- **Checked with the official client** `openai` 3.46.0 for Python (2026-11-08, `scripts/probar_openai.py`):
  `models.list()`, a whole answer, `stream=True` with `include_usage`, and errors arrive as the exceptions the client
  expects (`BadRequestError` for «tools», `AuthenticationError`, `NotFoundError`), with our message in Spanish.
- **Not checked yet** with tools such as Continue or Open WebUI. Until they are tested, compatibility with them is not
  announced.
- Tools that run **inside a browser** send `Origin` and are rejected (ADR 0015); those that talk from a server or an
  extension are not.

## Consequences

- When `tools` or images are added in the future, their rejection must be removed and tests added.
- `/v1` requests without `stream` wait for the whole answer: the client needs a long enough timeout.
