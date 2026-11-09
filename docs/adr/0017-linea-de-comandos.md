# ADR 0017 · Línea de comandos: cliente de la API, arranque bajo demanda y lexopt

- **Estado:** aceptada
- **Fecha:** 2026-11-09

## Decisión

- La CLI (`locall-cli`) es un **cliente de la API local**, como lo será la app de escritorio: nunca toca el núcleo
  directamente (regla 9). Lee el puerto de `config.toml` y el token de `%LOCALAPPDATA%\Locall\token`.
- **Arranque bajo demanda** (decisión ratificada): si Locall no está abierto, las órdenes que lo necesitan lanzan
  `locall serve` en segundo plano y esperan a que responda (hasta 20 s; si se cierra antes, se dice y se señala el
  registro). `status` y `stop` nunca lo abren.
- El núcleo en segundo plano se lanza:
  - **sin consola** (`DETACHED_PROCESS`) y en **su propio grupo** (`CREATE_NEW_PROCESS_GROUP`): un Ctrl+C en la
    terminal no le llega;
  - **fuera del Job Object** de la terminal si esta lo permite (`CREATE_BREAKAWAY_FROM_JOB`), para que no muera al
    cerrarla; si no lo permite, se lanza igualmente;
  - **sin heredar la entrada y salida** de la consola (ver Motivos).
- `locall stop` usa `POST /api/shutdown`, que cierra el núcleo igual que Ctrl+C: ordenadamente y parando el motor.
- Las órdenes se leen con **lexopt** (MIT, sin dependencias). La ayuda y todos los errores son nuestros, en español.
  Código de salida: 0 = bien, 1 = error, 2 = orden mal escrita.

## Motivos

- Un único camino (la API) para todos los clientes: la seguridad y las pruebas de la API cubren también la CLI.
- **Herencia de la salida:** en Windows, un proceso hijo hereda los "tubos" de la consola del padre. Con la primera
  versión, el núcleo heredaba el de la CLI y la terminal esperaba a que se cerrara: `locall list` tardaba **90 s** en
  terminar (hasta que el núcleo se cerraba por inactividad). Marcando la entrada y salida de la CLI como no heredables
  (`SetHandleInformation`) baja a **0,12 s**.
- **Sonda corta:** en Windows, conectar a un puerto cerrado de `localhost` tarda ~2 s porque el sistema reintenta.
  Para saber si Locall está abierto se usa una espera de conexión de 300 ms; si el servidor existe, el sistema acepta
  la conexión al instante aunque el programa esté ocupado. `status` con Locall cerrado pasa de 2 s a 0,3 s.
- **lexopt frente a clap:** clap es el más usado, pero sus mensajes de error están en inglés y cambiarlos todos es
  complicado. clap mínimo trae 4 dependencias; con `derive`, 21. lexopt, ninguna: solo resuelve lo difícil
  (`--opcion=valor`, `-abc`, argumentos no Unicode).

## Consecuencias

- Si en el puerto configurado hay otro programa, Locall no arranca y la CLI lo dice con la ruta del registro.
- Si el núcleo se cierra por inactividad entre dos órdenes, la siguiente lo vuelve a abrir (se nota un momento la
  primera vez).
- Hay que mantener la ayuda (`ordenes::AYUDA`) a mano al añadir órdenes.
