# ADR 0001 · Registrar las decisiones de arquitectura

- **Estado:** aceptada
- **Fecha:** 2026-11-08

## Contexto

Locall es un proyecto personal, sin fechas, que puede pausarse durante meses. Las decisiones importantes se tomaron
con datos (Fase 0) y hay que poder recordar **qué** se decidió y **por qué**.

## Decisión

Cada decisión técnica importante se documenta en un ADR corto dentro de `docs/adr/`, numerado (`NNNN-titulo.md`),
en español, con estas partes: estado, fecha, contexto, decisión, motivos, alternativas descartadas y consecuencias.

Un ADR no se borra: si una decisión cambia, se escribe uno nuevo que lo **sustituye** y el antiguo pasa a
"sustituida por NNNN".

## Motivos

- Regla 10 del proyecto: documentar las decisiones técnicas importantes.
- Facilita retomar el proyecto y explicar las decisiones a futuros colaboradores.

## Consecuencias

- Un poco más de trabajo al decidir; mucho menos al recordar.
- Las mediciones de la Fase 0 están en el informe de la Fase 0 (documentación del proyecto, fuera del repositorio);
  los ADR citan los datos clave.
