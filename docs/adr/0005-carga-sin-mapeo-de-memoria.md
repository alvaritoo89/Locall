# ADR 0005 · Load models without memory mapping (D2)

- **Status:** accepted
- **Date:** 2026-11-08

## Decision

Locall launches `llama-server` with `--load-mode none` (no `mmap`) by default on CPU and on integrated GPUs.

## Rationale (measurements on the reference machine, build 12189)

| Model | RAM with mmap | RAM without mmap | Speed |
|---|---|---|---|
| Qwen3 4B Q4_K_M (2.3 GB) | 4.18 GB | 2.53 GB | same or better (12.9 vs 12.4 t/s) |
| Qwen2.5 7B Q4_K_M (4.4 GB) | over 7.2 GB (cancelled, out of RAM) | 4.59 GB | 7.6 t/s |

- With mmap, memory use was 1.6 to 1.8 times the file size; without it, 1.05 to 1.1 times.
- Load time of the 4B (file in the OS cache): 2.29 s with mmap vs 1.96 s without.
- Ollama disables mmap when RAM is tight (`maybeDisableMmapForHostPressure`).

## Rejected alternatives

- **mmap by default (`auto`):** uses too much RAM on 16 GB machines.

## Consequences

- The memory estimate can be based on "file size + KV cache + margin" (measured error ~5%).
- Pending: measure a "cold" load (file not in the Windows cache).
- The old `-mmp` option no longer exists; `--load-mode` (`-lm`) is used.
