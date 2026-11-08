# ADR 0011 · Errores comprensibles del motor

- **Estado:** aceptada
- **Fecha:** 2026-11-08

## Decisión

- `locall-engine` reconoce los fallos más comunes de `llama-server` y los convierte en una `Causa` con un
  **mensaje propio en español**: puerto ocupado, memoria insuficiente, modelo dañado o incompleto, arquitectura
  no soportada, contexto excedido, modelo cargando y clave rechazada.
- **Fuentes:** para los fallos de carga, el final del registro del motor (últimos 64 KB). Para los errores de una
  petición, el campo `type` del JSON de error, que es más estable que el mensaje.
- **Nunca se muestra el texto del motor** al usuario: puede llevar la ruta del modelo, con el nombre de usuario de
  Windows (D4). Si la causa es desconocida, se dice qué falló y dónde está el registro.
- Única excepción a "clasificar solo para mensajes" (D8): si la causa es **puerto ocupado**, `arrancar_listo`
  reintenta con otro puerto (hasta 3 veces). Si una versión futura cambiara ese texto, simplemente no se reintentaría
  y se vería el error. No hay riesgo de hacer algo incorrecto.

## Motivos

- La Fase 0 mostró que los mensajes del motor son técnicos y en inglés ("failed to allocate buffer for kv cache").
- Los textos buscados se han comprobado **provocando cada fallo** con la build b12189. Las pruebas unitarias guardan
  esas líneas reales, así que un cambio de versión del motor que las rompa se notará.
- El puerto se elige libre justo antes de lanzar el motor, pero otro programa puede ocuparlo en ese instante; con el
  reintento, el usuario no llega a verlo.

## Alternativas descartadas

- **Mostrar el mensaje original del motor:** incomprensible para el usuario y con datos privados.
- **Pasar al motor un socket ya abierto** (y evitar así la carrera por el puerto): `llama-server` no lo admite.

## Consecuencias

- Al actualizar la build de llama.cpp hay que repetir las pruebas de fallos reales (`tests/motor_real.rs`,
  `tests/chat_real.rs`) por si cambian los textos.
- No verificado todavía: los fallos propios de Vulkan (memoria de la gráfica). Hoy se clasifican como desconocidos,
  hasta que los provoquemos y los reconozcamos.
