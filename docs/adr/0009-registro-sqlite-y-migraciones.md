# ADR 0009 · Registro de modelos en SQLite, con migraciones

- **Estado:** aceptada
- **Fecha:** 2026-11-08

## Decisión

- El registro de modelos instalados vive en una base de datos **SQLite** (`locall.db`), usada mediante `rusqlite`
  con SQLite **incluido** en el binario (`bundled`, SQLite 3.54.1).
- Modo **WAL**, claves foráneas activadas y espera de hasta 5 s si otro proceso está escribiendo.
- El esquema lleva versión (`PRAGMA user_version`). Las **migraciones** se aplican en orden, cada una en su
  transacción. **Una migración publicada nunca se modifica**: los cambios van en una nueva.
- Si la base de datos es de una versión **más nueva** de Locall, no se toca y se pide actualizar.
- Tablas `STRICT`; fechas como segundos desde 1970 (UTC); al leer, los datos se validan con los tipos de
  `locall-store` (`Huella`, `Segmento`, `NombreArchivo`).

## Motivos

- Transacciones: registrar o eliminar un modelo es todo o nada.
- Varios procesos (núcleo, CLI) pueden consultarla a la vez.
- Ollama tuvo que mantener dos formatos de almacenamiento a la vez por no haber versionado desde el principio.
- `bundled` evita depender de que el usuario tenga SQLite instalado.

## Alternativas descartadas

- **Archivos JSON:** sin transacciones; riesgo de corrupción si se corta la luz.
- **Otras bases de datos embebidas (sled, redb…):** menos conocidas y sin las herramientas de SQLite para
  inspeccionar los datos.

## Consecuencias

- Dependencias: `rusqlite` y `libsqlite3-sys` (MIT), SQLite (dominio público); en compilación, `cc` y similares
  (MIT/Apache, no se distribuyen).
- La primera compilación necesita las Build Tools de Microsoft (compila el código C de SQLite).
