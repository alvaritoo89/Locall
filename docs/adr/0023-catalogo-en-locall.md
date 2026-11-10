# ADR 0023 · El catálogo dentro de Locall: actualizar, instalar por nombre y recomendar

- **Estado:** aceptada
- **Fecha:** 2026-11-10

## Decisión

- **Al abrir**, el núcleo usa el catálogo más nuevo (`published`) entre el que lleva dentro y uno descargado antes
  (`<datos>\catalogo\`), siempre con la firma comprobada. Si el guardado no pasa la firma, se usa el incluido.
- **Actualizar** (`POST /api/catalog/update`): se mira en GitHub
  (`https://raw.githubusercontent.com/alvaritoo89/Locall/main/catalogo/catalogo.json` y su `.minisig`) **como mucho una
  vez al día** (salvo `force`). La hora de la última vez se guarda aunque falle, para no insistir si no hay internet.
  - Límites: 4 MB el catálogo, 4 KB la firma.
  - Solo se acepta si la firma es buena **y** `published` es mayor que el del que hay: un catálogo antiguo bien firmado
    se ignora (anti-retroceso).
  - Se guarda con archivo temporal + renombrar; si se corta a medias, al abrir la firma no cuadra y se usa el incluido.
  - Nada se mira solo: la app pide la actualización la primera vez que se abre Explorar; la CLI, con `locall catalog`.
- **Instalar por nombre** (`locall pull qwen3-4b`, `{"name": …}` en `POST /api/models/pull`): **antes de descargar**
  se comprueba que lo que ofrece Hugging Face en ese commit coincide exactamente con lo firmado (mismas partes, mismos
  tamaños, mismos sha256). Si no, error `catalog_mismatch` y no se descarga nada. Después, la instalación normal
  (comprueba la cabecera y verifica cada archivo con sha256).
- **Identificador** del modelo instalado: el nombre del catálogo (`qwen3-4b`) para la variante por defecto y
  `nombre-variante` (`qwen3-4b-q8_0`) para las demás. Así `locall run qwen3-4b` funciona tras `locall pull qwen3-4b`.
- En la CLI, `pull` con `/` sigue siendo un repositorio de Hugging Face; sin `/`, un nombre del catálogo.
- **¿Cabe?** en el catálogo es **aproximado**: el modelo no está descargado y no se puede leer su cabecera, así que se
  estima como el archivo + 10 % + 512 MB, contra la memoria libre ahora (como en la Biblioteca). La app y la CLI dicen
  «aprox.».
- La app muestra «Recomendados por Locall» al principio de Explorar, con la etiqueta «compatible» y la explicación de
  que no se han probado uno a uno (ADR 0022).

## Nuevas rutas y errores

| Ruta | Qué hace |
|------|----------|
| `GET /api/catalog` | Catálogo en uso: `published`, `source` (`builtin`/`downloaded`), `checked_at` y `models` (con `status: "compatible"`, `installed`, `fit_estimate`, `memory_estimate_bytes`, `size_bytes`, `variants`) |
| `POST /api/catalog/update` | `{"force"?}` → `{"result": "not_due" \| "no_changes" \| "updated", "catalog": …}` |

Errores: `not_in_catalog` (404), `catalog_mismatch` (409), `catalog_invalid` y `catalog_unavailable` (502).

## Privacidad

Ampliación del ADR 0008: además de Hugging Face, Locall se conecta a **GitHub** (raw.githubusercontent.com), como
mucho una vez al día y solo al usar Explorar o `locall catalog`, para descargar dos archivos públicos. No envía nada
más que la petición. La pantalla de Ajustes lo dice.

## Pruebas

Catálogos de prueba firmados con la clave de prueba (`crates/locall-core/tests/datos/`), publicados después del
oficial, contra un Hugging Face falso que también sirve el catálogo: actualizar una vez al día, guardar y recordar,
anti-retroceso, firma manipulada, GitHub sin el archivo, instalar por nombre, huella distinta en Hugging Face (no se
descarga nada) y nombre inexistente. Y una prueba (`#[ignore]`, usa internet) con el catálogo real de GitHub.
