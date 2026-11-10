# ADR 0022 · Catálogo de modelos probados, firmado con minisign

- **Estado:** aceptada
- **Fecha:** 2026-11-10

## Decisión

- Locall tiene un **catálogo propio de modelos probados** (`catalogo/catalogo.json` en este repositorio). Es la
  única fuente para la app, la CLI y la web.
- **Estar en el catálogo significa «probado»:** cada variante se instaló desde su commit exacto, el motor (la versión
  que indica `tested_engine`) la cargó y respondió. No se publican velocidades: dependen del equipo, y una cifra sin
  su contexto engañaría (regla 19). Cada persona ve en la app si el modelo cabe en **su** equipo.
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
`default`, `repo`, `commit`, `tested_engine`, `files`: `path`, `size`, `sha256`).

`locall pull qwen3-4b` instala la variante por defecto; `locall pull qwen3-4b:q8_0`, otra.

## Cómo se publica un catálogo nuevo

1. Editar `catalogo/catalogo.json` (subir `published`).
2. Firmarlo en el equipo de quien publica (PowerShell):

   ```powershell
   & "<ruta>\minisign.exe" -S -s "$env:USERPROFILE\.minisign\locall-catalogo.key" -m catalogo\catalogo.json
   ```

3. Subir los dos archivos (`catalogo.json` y `catalogo.json.minisig`).

## Consecuencias

- Si se pierde la clave privada (o su contraseña), no se pueden firmar catálogos nuevos hasta publicar una versión de
  Locall con otra clave pública. Por eso hay que guardar una copia de seguridad.
- Las pruebas usan **claves de prueba** propias (`crates/locall-catalogo/tests/datos/`); sus claves privadas no están
  en el repositorio.
- El `minisign.exe` 0.12 usado para firmar se comprobó con la firma de su autor (clave
  `RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3`, publicada en su README).
