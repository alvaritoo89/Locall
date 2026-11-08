# ADR 0016 · Compatibilidad con OpenAI y cierre por inactividad

- **Estado:** aceptada
- **Fecha:** 2026-11-08

## Decisión

### Compatibilidad con la API de OpenAI

- `GET /v1/models` y `POST /v1/chat/completions`, con el formato de OpenAI: `chat.completion` entero o, con
  `"stream": true`, trozos `chat.completion.chunk` por **SSE** terminados en `data: [DONE]`. Admite
  `stream_options.include_usage`, `max_tokens` / `max_completion_tokens`, `temperature`, `seed`, el rol `developer`
  y el contenido como lista de partes de texto.
- La "API key" es el **token de Locall** (los SDK de OpenAI ya envían `Authorization: Bearer …`). Se aplican las
  mismas protecciones que al resto (ADR 0015): `Host` y `Origin`.
- Lo que **no se admite todavía se rechaza** con un error 400 claro, en vez de ignorarlo: `tools`/`functions`,
  `tool_choice`, `response_format`, `n` > 1, imágenes en los mensajes y el rol `tool`.
- Los parámetros de muestreo que Locall todavía no pasa al motor (`top_p`, `presence_penalty`,
  `frequency_penalty`, `stop`…) **se ignoran**.
- Los errores mantienen nuestro formato `{"error": {"code", "message"}}`, que incluye el `message` que leen los
  clientes de OpenAI.

### Cierre por inactividad

- Ajuste `cierre_inactividad_minutos` (30 por defecto, 0 = nunca): si Locall lleva ese tiempo **sin peticiones y sin
  trabajo en marcha**, se cierra solo, parando el motor. La CLI y la app lo volverán a abrir cuando haga falta
  (decisión ratificada: núcleo bajo demanda, sin arrancar con Windows).
- Cuenta como trabajo en marcha: una instalación, una carga o una respuesta en curso, aunque no lleguen peticiones
  nuevas. Un modelo cargado sin usar no cuenta. Se comprueba cada 30 s.
- `locall serve` termina con Ctrl+C o por inactividad, lo que llegue antes.

## Motivos

- Ser compatible con OpenAI hace que funcionen sin cambios muchas herramientas y bibliotecas que ya existen.
- Rechazar lo no admitido evita que una herramienta crea que, por ejemplo, ha recibido una llamada a una función
  cuando Locall la ha ignorado (regla 12: no ocultar errores).
- Un núcleo que se cierra solo no se queda en segundo plano gastando memoria cuando el usuario ya no lo usa.

## Verificación y límites (regla 19)

- **Comprobado** con pruebas que siguen el formato documentado de OpenAI, contra el motor real: respuesta entera,
  SSE con uso al final y errores.
- **Comprobado con el cliente oficial** `openai` 3.46.0 de Python (2026-11-08, `scripts/probar_openai.py`):
  `models.list()`, respuesta entera, `stream=True` con `include_usage`, y los errores llegan como las excepciones
  que espera el cliente (`BadRequestError` con «tools», `AuthenticationError`, `NotFoundError`), con nuestro mensaje
  en español.
- **No comprobado todavía** con herramientas como Continue u Open WebUI. Hasta probarlas, no se anuncia
  compatibilidad con ellas.
- Las herramientas que se ejecutan **dentro de un navegador** envían `Origin` y se rechazan (ADR 0015); las que
  hablan desde un servidor o una extensión, no.

## Consecuencias

- Al añadir `tools` o imágenes en el futuro, habrá que quitar su rechazo y añadir pruebas.
- Las peticiones `/v1` sin `stream` esperan la respuesta entera: el cliente debe tener un tiempo de espera suficiente.
