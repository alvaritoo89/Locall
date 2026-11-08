# ADR 0004 · Solo GGUF, Windows primero, paquetes CPU y Vulkan

- **Estado:** aceptada
- **Fecha:** 2026-11-08

## Decisión

- La versión 1 solo admite modelos en formato **GGUF** (versión 3), descargados de Hugging Face con revisión
  (commit) y sha256 fijados, o importados del disco como "no verificados".
- Plataforma inicial: **Windows x64**. macOS y Linux, más adelante.
- Motor incluido: paquetes **CPU** y **Vulkan** de llama.cpp. Vulkan en gráficas dedicadas se marca como
  **experimental** hasta tener pruebas reales (D6). CUDA queda fuera de la v1.

## Motivos

- GGUF es autocontenido (metadatos, tokenizador, plantilla de chat) y es el formato de llama.cpp.
- Rechazar otros formatos elimina el riesgo de formatos que ejecutan código al cargarse (pickle).
- El equipo de referencia es Windows con gráfica integrada Intel; no se puede probar CUDA ni ROCm.
- Regla 19: no afirmar compatibilidad sin verificarla.

## Alternativas descartadas

- **Safetensors/ONNX/MLX en la v1:** exigirían otros motores; se reconsiderará con una necesidad concreta.
- **Incluir CUDA:** sin hardware para probarlo y con condiciones de redistribución propias de NVIDIA.

## Consecuencias

- Catálogo y validador centrados en GGUF; el lector de cabeceras GGUF es un componente crítico (con pruebas y
  fuzzing).
