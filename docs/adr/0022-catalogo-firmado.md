# ADR 0022 · Catálogo de modelos compatibles, firmado con minisign

- **Estado:** aceptada
- **Fecha:** 2026-11-10

## Decisión

- Locall tiene un **catálogo propio de modelos compatibles** (`catalogo/catalogo.json` en este repositorio). Es la
  única fuente para la app, la CLI y la web.
- **Estar en el catálogo significa «compatible»:** se leyó la cabecera del archivo exacto (fijado a su commit) y el
  motor que indica `checked_engine` reconoce su arquitectura y sus tipos de datos. **No se descargan ni se prueban**
  los modelos (decisión del autor, 2026-11-10): por eso no se dice «probado» en ningún sitio (regla 19). Que el
  formato sea compatible es una buena señal, no una garantía: un modelo de arquitectura muy nueva puede fallar al
  cargar con esta versión del motor. No se publican velocidades. Cada persona ve en la app si el modelo cabe en
  **su** equipo.
- Más adelante (Fase 7 u 8) se podrán probar los modelos de verdad en GitHub Actions, sin descargas en el equipo de
  quien publica, y entonces distinguir «compatible» de «probado».
- **Firmado con minisign** (Ed25519). La clave privada la guarda **solo quien publica**, en su equipo y con
  contraseña; nunca está en GitHub ni en ningún servidor. La clave pública va dentro de Locall
  (`locall_catalogo::CLAVE_PUBLICA`, id `5C09624CD1482FE0`).
- La verificación usa **`minisign-verify` 0.3.0** (MIT, sin dependencias, sin `unsafe`; la usa también el actualizador
  oficial de Tauri, así que servirá para las actualizaciones de la Fase 8). Se descartó **Sigstore**: 44 dependencias
  directas (tokio, reqwest, openssl…) y un registro público en internet en cada firma.
- **Primero la firma, después el contenido:** nunca se interpreta un catálogo sin firmar. Solo se aceptan firmas del
  formato actual de minisign (con BLAKE2b); el formato antiguo se rechaza.
- **Contra catálogos antiguos:** `published` va dentro de lo firmado. La app no aceptará nunca un catálogo más antiguo
  que el último que aceptó (así nadie puede devolver un catálogo viejo, bien firmado, con un modelo retirado).
- **Validación estricta** con límites (4 MB, 500 modelos, 16 variantes, 64 archivos…): nombres, commits de 40
  caracteres, sha256 de 64, rutas `.gguf` relativas sin `..`. Los campos desconocidos se ignoran; un `format` más
  nuevo pide actualizar Locall.
- `catalogo/**` se guarda en Git **byte a byte** (`-text` en `.gitattributes`): si Git cambiara los saltos de línea,
  la firma dejaría de valer.

## Formato

Ver la documentación de `crates/locall-catalogo/src/lib.rs`. Nombres en inglés, como la API: `format`, `published` y
`models` (`name`, `title`, `publisher`, `description`, `tags`, `license`, `context_length` y `variants`: `name`,
`default`, `repo`, `commit`, `checked_engine`, `files`: `path`, `size`, `sha256`).

`locall pull qwen3-4b` instala la variante por defecto; `locall pull qwen3-4b:q8_0`, otra.

## Cómo se publica un catálogo nuevo

1. Editar `catalogo/catalogo.json` (subir `published`).
2. Firmarlo en el equipo de quien publica (PowerShell):

   ```powershell
   & "<ruta>\minisign.exe" -S -s "$env:USERPROFILE\.minisign\locall-catalogo.key" -m catalogo\catalogo.json
   ```

3. Subir los dos archivos (`catalogo.json` y `catalogo.json.minisig`).

## Catálogo v1 (2026-11-10)

12 modelos, todos «compatibles» con el motor b12189 (cabecera del archivo exacto leída; arquitecturas qwen3, gemma4,
mistral3, granite, llama, phi3 y gpt-oss) y ninguno descargado: Qwen3 1.7B, 4B y 8B; Gemma 4 E2B y E4B; Ministral 3
3B y 8B; Granite 4.2 3B y 8B; SmolLM2 1.7B; Phi-4; gpt-oss 20B. Una variante por modelo (la cuantización Q4 que
publica cada autor). Todos públicos, sin cuenta, con licencia Apache-2.0 (Phi-4: MIT). Firmado por el autor con la
clave `5C09624CD1482FE0`. Los modelos de Gemma 4 y Ministral 3 también entienden imágenes; Locall los usa solo para
texto.

Locall lleva dentro una copia (`locall_catalogo::incluido()`), verificada igual que una descargada; una prueba falla
si alguien edita el catálogo sin volver a firmarlo.

## Consecuencias

- Si se pierde la clave privada (o su contraseña), no se pueden firmar catálogos nuevos hasta publicar una versión de
  Locall con otra clave pública. Por eso hay que guardar una copia de seguridad.
- Las pruebas usan **claves de prueba** propias (`crates/locall-catalogo/tests/datos/`); sus claves privadas no están
  en el repositorio.
- El `minisign.exe` 0.12 usado para firmar se comprobó con la firma de su autor (clave
  `RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3`, publicada en su README).
