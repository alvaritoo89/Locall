# ADR 0002 · Rust para el núcleo

- **Estado:** aceptada
- **Fecha:** 2026-11-08

## Decisión

El núcleo de Locall (gestión de modelos, motor, API local y CLI, en un único binario) se escribe en **Rust**.
La aplicación de escritorio usará Tauri 2, cuyo backend también es Rust.

## Motivos

- Un solo lenguaje para el núcleo y el shell de escritorio.
- Seguridad de memoria y gestión de errores explícita (`Result`), adecuadas para un proceso que corre durante horas.
- Buen acceso a la API de Windows (Job Objects, demostrados en la Fase 1 con `windows-sys`).
- Objetivo de aprendizaje del autor: aprender Rust y programación de sistemas.

## Alternativas descartadas

- **Go:** probado en producción por Ollama y más fácil de aprender; descartado por el objetivo de aprendizaje y por
  unificar con Tauri. Sigue siendo una alternativa válida si Rust se convirtiera en un freno.

## Consecuencias

- Curva de aprendizaje alta al principio: se compensa avanzando paso a paso, con explicaciones y pruebas.
- El uso de `unsafe` se limita a módulos pequeños de integración con el sistema, cada bloque justificado.
