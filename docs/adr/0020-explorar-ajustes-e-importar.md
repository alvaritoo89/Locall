# ADR 0020 · Explorar, Ajustes e importar desde la app

- **Estado:** aceptada
- **Fecha:** 2026-11-10

## Decisión

### Explorar: buscar en Hugging Face por nombre

- El núcleo busca con `GET https://huggingface.co/api/models?search=…&filter=gguf&sort=downloads&direction=-1`
  (formato comprobado en vivo el 2026-11-10): solo repositorios con GGUF, los más descargados primero, como mucho 50.
- **A Hugging Face solo le llega el texto buscado** (codificado), sin cuenta ni datos del equipo. Esto **amplía el
  ADR 0008**: además de descargar, Locall consulta Hugging Face al buscar, y solo cuando la persona busca. La pantalla
  lo dice.
- La API lo ofrece como `POST /api/hub/search` con `{"query", "limit"?}`: con el texto en el cuerpo, lo buscado nunca
  forma parte de una dirección (ni aparece en una línea de petición), y no hace falta decodificar direcciones.
- Al abrir un resultado se consulta el repositorio y se instala igual que desde la Biblioteca (versión fijada al
  commit, compatibilidad comprobada antes de descargar, sha256).
- Las instalaciones en marcha se guardan en la raíz de la interfaz (`App`), no en cada pantalla: el progreso se ve en
  Explorar y en la Biblioteca y no se pierde al cambiar de pantalla.

### Ajustes

- `GET /api/settings` da los ajustes **en uso** (leídos al abrir Locall) y los **guardados** (`config.toml` ahora), y
  si hace falta reiniciar. `PATCH /api/settings` guarda cambios.
- **Los cambios se aplican al reiniciar Locall**, no en caliente: el núcleo lee los ajustes al arrancar y cambiarlos
  en marcha (el contexto de un modelo cargado, el tiempo de descarga del planificador…) sería frágil. La app ofrece
  «Reiniciar Locall ahora» (`POST /api/shutdown`; se vuelve a abrir solo), desactivado mientras algo está en marcha.
- **Se pueden cambiar** el contexto, los hilos, los minutos para liberar la memoria y para cerrar Locall, y los días de
  registros. **No se cambian desde la app** (sí a mano en `config.toml`, con Locall cerrado):
  - la **carpeta de modelos**: los modelos instalados se quedarían en la antigua; moverlos bien es trabajo aparte;
  - el **puerto**: la app y la CLI buscan a Locall en el puerto del archivo; cambiarlo con Locall abierto acabaría con
    dos Locall abiertos;
  - **paralelo**: fijado en 1 (ADR 0012).
- **Guardar respeta el archivo del usuario:** solo se reescriben las líneas de los ajustes cambiados (un comentario al
  final de esa misma línea se pierde; los demás se quedan), el resultado se valida antes de escribirlo y se escribe en
  un archivo temporal que después sustituye al original.

### Importar un `.gguf` desde la app

- Se abre la ventana normal de Windows con **`rfd` 0.17.2** (MIT). Es la pieza que usa por dentro el complemento
  oficial `tauri-plugin-dialog`, pero sin él: el complemento arrastra `tauri-plugin-fs` y obliga a dar permisos de
  archivos a la interfaz. En Windows, `rfd` solo usa `windows-sys` y `raw-window-handle`, que ya estaban en el
  proyecto: añade **un** crate. Se eligió la 0.17.2 y no la 0.18.0 porque esta tenía 3 días.
- **El diálogo lo abre la parte Rust**, y la ruta elegida va directa a la API: la interfaz nunca da una ruta. Lo mismo
  al abrir las carpetas en el Explorador: la interfaz solo elige «datos» o «modelos» y la ruta la da Locall.

## Consecuencias

- La búsqueda depende de la API pública de Hugging Face; si cambia su formato, Explorar fallará con un error claro
  (hay una prueba contra el Hugging Face real, marcada `#[ignore]` porque usa internet).
- Mientras no haya catálogo propio (Fase 6), Explorar muestra lo que hay en Hugging Face sin filtrar por calidad.
- Reiniciar descarga el modelo de la memoria: la siguiente respuesta tarda unos segundos más en empezar.
