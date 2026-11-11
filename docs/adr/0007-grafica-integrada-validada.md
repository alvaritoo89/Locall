# ADR 0007 · Integrated GPU enabled only on validated devices (D1)

- **Status:** accepted
- **Date:** 2026-11-08

## Decision

Integrated-GPU acceleration (Vulkan) is enabled **by default only on validated devices** (today: an Intel integrated
GPU in a recent laptop), with a switch to turn it off and an **automatic fallback to CPU** if the engine fails with it.
On every other integrated GPU it is off by default.

"Use the CPU" means using the engine's **CPU package**: in the Vulkan package, `-ngl 0` still sends prompt processing
to the GPU.

## Rationale (Intel integrated GPU, Vulkan package, 8 threads, no mmap)

| Model | Prompt processing CPU → GPU | Generation CPU → GPU |
|---|---|---|
| 1.5B | 153 → 509 t/s | 32.8 → 29.7 t/s |
| 4B | 47.6 → 189 t/s | 12.9 → 12.9 t/s |
| 7B | 27.4 → 115 t/s | 7.6 → 7.5 t/s |

- Prompt processing is 3.3 to 4.2 times faster; generation is the same. The CPU runs cooler.
- "GPU" memory comes from the same RAM: Vulkan reports 8 GB "free" that is really system RAM.
- Ollama excludes non-CUDA integrated GPUs by default.
- Only one machine and one driver tested: no generalizing.

## Consequences

- A list of validated devices and a CPU fallback mechanism are needed.
- The memory rule never uses the free memory Vulkan reports on unified-memory GPUs.
