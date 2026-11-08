# ADR 0005 · Cargar los modelos sin mapeo de memoria (D2)

- **Estado:** aceptada
- **Fecha:** 2026-11-08

## Decisión

Locall lanza `llama-server` con `--load-mode none` (sin `mmap`) por defecto en CPU y en gráficas integradas.

## Motivos (mediciones en el equipo de referencia, build 12189)

| Modelo | RAM con mapeo | RAM sin mapeo | Velocidad |
|---|---|---|---|
| Qwen3 4B Q4_K_M (2,3 GB) | 4,18 GB | 2,53 GB | igual o mejor (12,9 frente a 12,4 t/s) |
| Qwen2.5 7B Q4_K_M (4,4 GB) | más de 7,2 GB (cancelado por falta de RAM) | 4,59 GB | 7,6 t/s |

- Con mapeo, la memoria usada era de 1,6 a 1,8 veces el tamaño del archivo; sin mapeo, de 1,05 a 1,1 veces.
- Tiempo de carga del 4B (archivo en caché del sistema): 2,29 s con mapeo frente a 1,96 s sin mapeo.
- Ollama desactiva el mapeo cuando la RAM va justa (`maybeDisableMmapForHostPressure`).

## Alternativas descartadas

- **Mapeo por defecto (`auto`):** consume demasiada RAM en equipos de 16 GB.

## Consecuencias

- La estimación de memoria puede basarse en "tamaño del archivo + caché KV + margen" (error medido de ~5 %).
- Pendiente: medir la carga "en frío" (archivo fuera de la caché de Windows).
- La opción antigua `-mmp` ya no existe; se usa `--load-mode` (`-lm`).
