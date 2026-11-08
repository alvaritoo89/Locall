# ADR 0003 · llama.cpp oficial como subproceso, con planificador propio

- **Estado:** aceptada
- **Fecha:** 2026-11-08

## Decisión

- El motor de inferencia es **`llama-server` de llama.cpp, binarios oficiales sin modificar**, con una compilación
  fijada y verificada por sha256 (hoy: build 12189).
- Se ejecuta como **proceso hijo**, nunca enlazado dentro del núcleo, y dentro de un **Job Object** de Windows con
  `KILL_ON_JOB_CLOSE`.
- Locall usa **su propio planificador** (v1: un modelo cargado a la vez) y **no** el modo router de llama-server.

## Motivos

- Ollama acabó usando el `llama-server` original como subproceso; Jan, aun compilándolo dentro de su programa, lo
  sigue ejecutando en un proceso aparte porque `GGML_ASSERT` termina el proceso y no se puede capturar.
- Fase 1: sin Job Object, matar el núcleo dejó un `llama-server` huérfano con 1,21 GB; con Job Object, Windows lo
  terminó solo. 10 ciclos seguidos sin huérfanos ni fugas de memoria.
- El modo router queda fuera del ámbito de seguridad de llama.cpp (`SECURITY.md`) y a Jan le dejó procesos
  huérfanos tras actualizar.

## Alternativas descartadas

- **Enlazar llama.cpp como biblioteca (FFI):** un fallo del motor tumbaría el núcleo y habría que compilar los
  backends (CUDA, Vulkan) nosotros.
- **Compilar nuestro propio motor (como Jan):** demasiada complejidad para empezar; la arquitectura permite
  reconsiderarlo.
- **Modo router de llama-server:** ver motivos.

## Consecuencias

- Dependemos del contrato HTTP y de las opciones de línea de comandos de llama-server: cada cambio de compilación se
  valida con pruebas de regresión antes de distribuirse.
- Los binarios oficiales no están firmados y su ZIP no incluye la licencia MIT de llama.cpp: Locall añadirá los
  avisos de licencia.
