# ADR 0012 · Una conversación a la vez por modelo, y cola por orden de llegada

- **Estado:** aceptada
- **Fecha:** 2026-11-08

## Decisión

- El motor se arranca con **`--parallel 1`** (`ConfigMotor::paralelo`, por defecto `PARALELO_POR_DEFECTO = 1`).
  Atiende una conversación cada vez; si llegan dos a la vez, la segunda espera dentro del motor y termina bien
  (comprobado con b12189).
- El planificador atiende las peticiones **por orden de llegada** (turnos):
  - quien pide el modelo ya cargado **no se cuela** por delante de quien espera otro modelo;
  - los que esperan seguidos el mismo modelo pasan juntos (una sola carga);
  - se puede **cancelar** mientras se espera turno (`usar_con`); el siguiente pasa enseguida.
- `paralelo` es configurable: más adelante (pieza 10) podrá subirse en equipos potentes o para clientes de la API
  que hagan varias peticiones a la vez.

## Motivos

- Por defecto, b12189 abre **4 huecos** que **comparten** el contexto (`kv_unified`): con 4 conversaciones largas a
  la vez, cada una tendría en la práctica una cuarta parte del contexto, y el usuario no entendería por qué.
- En un procesador de portátil, generar 4 respuestas a la vez hace **cada una más lenta**. Para el uso principal
  (una persona chateando), una sola conversación a toda velocidad es mejor.
- Con un hueco, el motor reutiliza mejor la caché de la conversación anterior.
- La memoria es la misma con 1 o con 4 huecos (el contexto es un único bloque compartido), así que la estimación de
  `locall-compat` sigue siendo válida.
- Sin orden de llegada, quien pidiera otro modelo podría esperar para siempre si siguen llegando peticiones del
  modelo cargado.

## Alternativas descartadas

- **4 huecos (lo que hace el motor por defecto):** contexto dividido y respuestas más lentas sin avisar.
- **Dar a cada hueco su propio contexto** (`--kv-unified-per-slot`): multiplica la memoria por el número de huecos.

## Consecuencias

- Dos peticiones simultáneas al mismo modelo se responden **una detrás de otra**. La API (Fase 3) debe informar de
  que la segunda está esperando.
- La cancelación en la cola se comprueba cada 50 ms (quien cancela solo cambia un `AtomicBool`).
- Una carga ya empezada no se interrumpe al cancelar: el motor no lo permite a medias de forma segura.
