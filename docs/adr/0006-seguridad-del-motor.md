# ADR 0006 · Seguridad del motor: clave, red y respuestas (D3, D4)

- **Estado:** aceptada
- **Fecha:** 2026-11-08

## Decisión

Al lanzar `llama-server`, Locall:

1. Lo hace escuchar **solo en `127.0.0.1`**, en un **puerto nuevo** elegido en cada arranque.
2. Genera una **clave aleatoria de 256 bits** en cada arranque y la pasa por la variable de entorno
   **`LLAMA_API_KEY`**, nunca como parámetro de línea de comandos.
3. Restringe CORS con `--cors-origins localhost` y desactiva la interfaz web (`--no-webui`).
4. **Nunca expone el puerto del motor ni reenvía sus respuestas tal cual:** el núcleo las traduce a su propio
   formato, con el nombre del modelo en lugar de su ruta.

## Motivos (contrato verificado en la Fase 0)

- Sin clave, `/v1/models`, `/props`, `/slots` y `/metrics` responden 401; `/health` responde sin clave.
- CORS por defecto: devuelve cualquier `Origin` con `Access-Control-Allow-Credentials: true`.
- `/v1/models` y cada fragmento del streaming incluyen la **ruta completa del modelo** (contiene el nombre de usuario
  de Windows).
- Jan pasa la clave por variable de entorno porque los parámetros de un proceso los puede leer cualquier programa del
  equipo.
- Tras una parada forzada, el puerto queda unos segundos en estado "Listen" huérfano.

## Consecuencias

- Fase 1 verificó que la clave no aparece en el registro del motor.
- La API pública de Locall tiene su propia autenticación (token local), independiente de esta clave.
