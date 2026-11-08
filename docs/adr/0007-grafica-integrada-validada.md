# ADR 0007 · Gráfica integrada activada solo en dispositivos validados (D1)

- **Estado:** aceptada
- **Fecha:** 2026-11-08

## Decisión

La aceleración por gráfica integrada (Vulkan) se activa **por defecto solo en dispositivos validados** (hoy:
Intel Iris Xe), con un interruptor para desactivarla y **vuelta automática a CPU** si el motor falla con ella.
En el resto de gráficas integradas queda desactivada por defecto.

"Usar CPU" significa usar el **paquete CPU** del motor: en el paquete Vulkan, `-ngl 0` sigue enviando la lectura del
prompt a la gráfica.

## Motivos (Iris Xe, paquete Vulkan, 8 hilos, sin mapeo)

| Modelo | Leer el prompt CPU → gráfica | Escribir CPU → gráfica |
|---|---|---|
| 1.5B | 153 → 509 t/s | 32,8 → 29,7 t/s |
| 4B | 47,6 → 189 t/s | 12,9 → 12,9 t/s |
| 7B | 27,4 → 115 t/s | 7,6 → 7,5 t/s |

- Leer el prompt es entre 3,3 y 4,2 veces más rápido; escribir va igual. El procesador se calienta menos.
- La memoria "de la gráfica" sale de la misma RAM: Vulkan informa de 8 GB "libres" que en realidad son RAM del sistema.
- Ollama descarta por defecto las gráficas integradas que no son CUDA.
- Un solo equipo y un solo driver probados: no se generaliza.

## Consecuencias

- Hace falta una lista de dispositivos validados y un mecanismo de vuelta a CPU.
- La regla de memoria nunca usa la memoria libre que informa Vulkan en gráficas con memoria unificada.
