# ADR 0008 · Privacidad local: sin telemetría

- **Estado:** aceptada
- **Fecha:** 2026-11-08

## Decisión

- Locall **no incluye telemetría ni analítica** de ningún tipo.
- Los únicos datos que salen del equipo son:
  - las **descargas de modelos** desde Hugging Face;
  - la **consulta del catálogo y de las actualizaciones** desde GitHub (desactivable).
- **Nunca** se envían conversaciones, prompts ni archivos del usuario.
- La aplicación mostrará un **registro de conexiones**: qué ha salido del equipo, cuándo y por qué.
- Las APIs externas (OpenAI, Anthropic…) solo existirán como función futura y opcional, con las claves de cada
  usuario guardadas en el Administrador de credenciales de Windows.

## Motivos

- Reglas 17 y 18 del proyecto: sin analítica invasiva; la privacidad local es un principio de diseño.
- Diferenciación frente a herramientas con telemetría activada por defecto o con servicios en la nube.
- Sin servidores propios, el proyecto se mantiene gratis.

## Consecuencias

- Los fallos se diagnostican con un **paquete de diagnóstico** que el usuario revisa y decide enviar.
- No habrá métricas de uso: las prioridades se deciden con issues y opiniones explícitas.
