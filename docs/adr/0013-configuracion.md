# ADR 0013 · Configuración: carpeta de datos y config.toml con lector propio

- **Estado:** aceptada
- **Fecha:** 2026-11-08

## Decisión

- Los datos de Locall viven en **`%LOCALAPPDATA%\Locall`**: `config.toml`, `locall.db`, `modelos\` y
  `registros\`. Con la variable `LOCALL_DATOS` se puede usar otra carpeta (pruebas, versión portátil).
- Los ajustes de la v1: `carpeta_modelos`, `hilos` (`"auto"` o número), `contexto`, `inactividad_minutos`,
  `paralelo` y `registros_dias`. Cada uno con un **rango validado** y mensajes que dicen la línea.
- El archivo usa un **subconjunto de TOML** (`clave = valor`, comentarios `#`, textos `'...'` y `"..."`, enteros)
  leído con un **lector propio** (`locall-config::lector`).
- Si el archivo no existe, se usan los valores por defecto. `crear_si_no_existe` lo crea con los valores por
  defecto comentados y **nunca sobrescribe** el del usuario.
- Una clave desconocida es un **aviso**, no un error.

## Motivos

- `Local` y no `Roaming`: la parte "Roaming" se puede sincronizar con otros equipos, y los modelos ocupan gigas.
- **Sin dependencias nuevas** (regla 3): con los crates `toml` + `serde` serían unas diez dependencias para leer seis
  ajustes planos. El lector propio tiene unas 150 líneas con pruebas. Como el formato es TOML válido, se puede
  cambiar a `toml` en el futuro **sin tocar los archivos de los usuarios**.
- Las rutas de Windows se escriben con comillas simples (`'D:\Modelos'`) porque en TOML las dobles interpretan `\`.
  Si alguien usa dobles, el mensaje lo explica.
- Avisar en vez de fallar ante claves desconocidas permite abrir con una versión antigua un archivo de una nueva.
- Los hilos se validan contra los del equipo: en la Fase 0, 246 hilos en un equipo de 12 hundieron la velocidad.

## Alternativas descartadas

- **`toml` + `serde`:** muchas dependencias para un archivo tan simple.
- **JSON (ya tenemos `serde_json`):** sin comentarios y fácil de romper a mano con una coma.
- **Registro de Windows:** difícil de editar, de copiar y de llevar a Linux.

## Consecuencias

- Si en el futuro hacen falta secciones o listas, se cambia al crate `toml` (mismo formato).
- El ajuste de la gráfica (Vulkan) se añadirá cuando esté validada (D1).
