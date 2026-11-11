# ADR 0004 · GGUF only, Windows first, CPU and Vulkan packages

- **Status:** accepted
- **Date:** 2026-11-08

## Decision

- Version 1 only supports models in **GGUF** format (version 3), downloaded from Hugging Face with a pinned revision
  (commit) and sha256, or imported from disk as "unverified".
- Initial platform: **Windows x64**. macOS and Linux later.
- Bundled engine: llama.cpp's **CPU** and **Vulkan** packages. Vulkan on dedicated GPUs is marked **experimental**
  until it has been tested for real (D6). CUDA is out of scope for v1.

## Rationale

- GGUF is self-contained (metadata, tokenizer, chat template) and is llama.cpp's format.
- Rejecting other formats removes the risk of formats that execute code when loaded (pickle).
- The reference machine is Windows with Intel integrated graphics; CUDA and ROCm cannot be tested.
- Rule 19: never claim compatibility without verifying it.

## Rejected alternatives

- **Safetensors/ONNX/MLX in v1:** they would require other engines; to be reconsidered given a concrete need.
- **Bundling CUDA:** no hardware to test it, and NVIDIA's own redistribution terms.

## Consequences

- The catalog and the validator focus on GGUF; the GGUF header reader is a critical component (with tests and
  fuzzing).
