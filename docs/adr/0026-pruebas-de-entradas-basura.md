# ADR 0026 · Fase 7b: pruebas de «entradas basura» (fuzzing) sin dependencias

- **Estado:** aceptada
- **Fecha:** 2026-11-11

## Decisión

- Todo lo que Locall lee **de fuera** se prueba con miles de entradas **válidas estropeadas a propósito**: cambiar
  bits y bytes, poner números enormes donde van tamaños, meter, quitar, repetir y cortar trozos. Debe terminar
  **siempre** con un resultado controlado (normalmente un error), sin:
  - cerrarse de golpe (`panic`);
  - reservar de una vez más de **16 MB** (lo mide un reservador de memoria propio, `MedidorDeMemoria`, instalado
    solo en los programas de prueba).
- Crate propio **solo para pruebas** (`crates/locall-mutador`), **sin dependencias**: generador xorshift64* con
  semilla fija (todo es reproducible) y, si una entrada falla, se guarda en un archivo con su número y su semilla.
- Se descartaron `cargo-fuzz` (necesita Rust *nightly* y no funciona bien en Windows) y `proptest`/`arbitrary`
  (dependencias para algo que se resuelve en ~200 líneas).
- `cargo test` prueba pocas entradas para seguir siendo rápido; `LOCALL_BASURA=<n>` hace pasadas largas.

## Qué se prueba

| Qué | Por qué es «de fuera» |
|-----|------------------------|
| Cabeceras GGUF (`locall-gguf`) | Los modelos son de cualquier autor de Hugging Face |
| Catálogo, su firma y la clave (`locall-catalogo`) | Llega de GitHub (o de quien se haga pasar por él) |
| Ajustes (`locall-config`) | `config.toml` se edita a mano |
| Trozos de respuesta, errores y registro del motor (`locall-engine`) | Un modelo malicioso podría hacer escribir cosas raras al motor |
| Peticiones de chat, conversaciones y compatibles con OpenAI (`locall-api`) | Cualquier programa del equipo con el token |

## Lo que encontraron

- **Fallo real, arreglado:** el lector GGUF reservaba la memoria que **decía** la longitud de un texto (hasta 64 MB)
  antes de comprobar que los datos existían. Un archivo de pocos KB hizo reservar 33,5 MB de golpe. Ahora reserva a
  medida que llegan los datos (empieza con 64 KB).
- **Comprobado, no es un problema:** `minisign-verify` no mira la marca de algoritmo de la clave (las dos primeras
  letras) e ignora las líneas que sobran al final de una firma. En los dos casos la clave y la firma criptográficas
  son idénticas; nunca acepta algo firmado con otra clave.

Pasadas largas (modo optimizado, 2026-11-11): 2 millones de cabeceras GGUF, 600.000 catálogos/firmas/claves,
1 millón de ajustes, 1,5 millones de entradas del motor y 1,5 millones de peticiones a la API. Sin fallos.

## Además (revisión de los `expect` y los índices)

Todos los `expect` e índices del código revisados están protegidos por una validación previa. Uno se mejoró: si la
ventana de la app no puede abrirse (por ejemplo, falta WebView2), la versión final ya no se cierra sin decir nada:
muestra una ventana de error de Windows con el motivo.
