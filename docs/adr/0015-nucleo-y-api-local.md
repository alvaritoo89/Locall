# ADR 0015 · Núcleo y API local (Fase 3)

- **Estado:** aceptada
- **Fecha:** 2026-11-08

## Decisión

### Organización (regla 9)

- **`locall-core`**: el núcleo, sin HTTP. Une almacén, registro, descargas, planificador y configuración en un
  `Nucleo` que se comparte entre hilos.
- **`locall-api`**: solo el servidor HTTP, la seguridad y la traducción a JSON.
- **`locall`**: el programa (`locall serve` en esta fase; el resto de órdenes en la Fase 4). Un solo ejecutable para
  el núcleo y la CLI (decisión ratificada en la fase de investigación).

### Instalar desde Hugging Face

1. Se resuelve la revisión pedida (`main`, una etiqueta…) a su **commit exacto** y se usa ese commit en todo lo
   demás.
2. La lista de archivos de ese commit da el tamaño y el **sha256** (`lfs.oid`) de cada GGUF. Un archivo sin sha256
   **no se descarga**: no se podría verificar.
3. Antes de descargar, se lee **solo la cabecera** del GGUF por la red y se comprueba la compatibilidad
   (`locall-compat`): un modelo que el motor no puede usar no se descarga.
4. Los modelos divididos se instalan con todas sus partes, que conservan su nombre original.
5. Los repositorios protegidos (`gated`) se rechazan con un mensaje claro: la v1 no usa cuenta ni token de Hugging
   Face.

### Importar del disco

Un GGUF que ya está en el disco se incorpora con un **enlace duro** si está en el mismo disco que el almacén (no
ocupa el doble) o con una copia si no. Al eliminar el modelo de Locall, el archivo original del usuario no se toca.

### Eliminar

Se borran el registro, la carpeta del modelo y los archivos del almacén que **ningún otro modelo** use. Si el
modelo está cargado y nadie lo usa, se descarga de la memoria; si se está usando, no se elimina.

### API (partes 3b–3e)

- Servidor con **axum + tokio**. El núcleo es de hilos normales y se llama con `spawn_blocking`.
- Solo en **127.0.0.1**, con un **token** guardado en `%LOCALAPPDATA%\Locall\token`. Se rechazan las peticiones con
  un `Host` que no sea `localhost` o `127.0.0.1` (contra el "DNS rebinding") y las que vengan de webs no permitidas
  (`Origin`).
- **Rutas y campos en inglés** (API para programadores; la parte compatible con OpenAI lo exige); mensajes para
  personas en español, con un código fijo para los programas.

## Motivos

- **axum** frente a tiny_http (sin versiones desde octubre de 2022) o hyper "a pelo": está mantenido, es el más usado,
  y su lector HTTP lo ha revisado mucha más gente que cualquier cosa nuestra (seguridad, regla 1). Trae 28
  dependencias nuevas (todas MIT/Apache, más una MIT+BSD-3), pero tokio llegará igualmente con la app de escritorio
  (Tauri).
- Fijar el commit y exigir sha256 hace las instalaciones **reproducibles y verificables**.
- Leer solo la cabecera evita descargar gigas inútiles.

## Consecuencias

- `verificado = false` en todo lo instalado hasta que exista el catálogo verificado.
- Las licencias de terceros con atribución obligatoria (BSD-3) deberán aparecer en un archivo de avisos al
  distribuir.
- Si un modelo se elimina justo mientras otro hilo empieza a cargarlo, la carga fallará con un error claro. Es una
  ventana muy pequeña y no corrompe nada.
