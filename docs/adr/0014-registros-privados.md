# ADR 0014 · Registros: lo que hace Locall, nunca lo que dices

- **Estado:** aceptada
- **Fecha:** 2026-11-08

## Decisión

- Cada crate de Locall escribe con la fachada estándar **`log`** (`log::info!`, `log::warn!`). Un escritor propio
  (`locall-config::registros`) lo guarda en **`registros\locall-AAAA-MM-DD.log`**, un archivo por día con la hora
  local de Windows (`GetLocalTime`).
- Solo se guardan los mensajes de **nuestros crates** (`locall…`) de nivel **INFO o más importante**. Los de otros
  crates (`ureq`…) y los de depuración no se guardan.
- Al iniciar se borran los `locall-*.log` de más de `registros_dias` días (7 por defecto). Ningún otro archivo de la
  carpeta se toca.
- Escribir en el registro **nunca hace fallar a Locall**: si el disco falla, se pierde la línea.
- **Privacidad (regla 17):** se anota lo que **hace** Locall (modelo cargado o descargado y por qué, motor
  arrancado o parado, descargas, reintentos, errores, tiempos y cifras de tokens). **Nunca** se anota el contenido
  de las conversaciones (preguntas, respuestas, razonamientos), las claves ni los tokens de acceso.

## Motivos

- `log` ya estaba en el proyecto (la trae `ureq`) y es el estándar de Rust: **ninguna dependencia nueva**. El
  escritor propio son unas 100 líneas; `env_logger` o `tracing` traerían bastantes más dependencias.
- Un registro que contiene conversaciones es un riesgo: los usuarios lo comparten al pedir ayuda. Por eso la regla
  es "solo cifras", y una **prueba con el motor real** (`locall-scheduler/tests/registros_real.rs`) hace una pregunta
  con una palabra secreta y comprueba que ni la pregunta, ni la respuesta, ni la clave del motor aparecen.
- Filtrar por crate evita depender de lo que otros crates decidan anotar.

## Alternativas descartadas

- **`env_logger`, `tracing` o `flexi_logger`:** más dependencias para lo mismo.
- **Registrar en el Visor de eventos de Windows:** difícil de leer y de compartir para el usuario.
- **Fecha en UTC:** más fácil de calcular, pero confusa al leer el registro ("¿por qué pone las 19:00 si eran las
  20:00?").

## Consecuencias

- Quien añada un `log::info!` debe respetar la norma de privacidad; la prueba cubre el camino principal (cargar,
  conversar, descargar el modelo), no cada línea futura.
- Los mensajes de error que incluyen la ruta del registro del motor llevan la carpeta del usuario. Es aceptable
  porque el archivo es local, pero hay que tenerlo en cuenta si algún día se ofrece "enviar registros".
- El registro del motor (`llama-server`) es aparte: se sobrescribe en cada arranque y la limpieza no lo toca.
