# ADR 0018 · App de escritorio: Tauri 2, React y el token fuera de JavaScript

- **Estado:** aceptada
- **Fecha:** 2026-11-09

## Decisión

- La app (`crates/locall-app`) se hace con **Tauri 2** (2.12, estable; la 3 está en *alpha*). Usa el WebView2 que trae
  Windows, así que la app es pequeña.
- La interfaz, en `crates/locall-app/ui`, se hace con **React 19 + TypeScript + Vite**, con CSS propio (variables con
  los colores de las maquetas: blanco y negro con acento verde salvia `#5B7464`, `#8FA597` en oscuro). No usa
  librerías de componentes ni Tailwind.
- **La parte Rust de la app es un cliente más de la API** (regla 9): reutiliza el cliente y el arranque bajo
  demanda de `locall-cli`. La interfaz no habla nunca con la API; pide las cosas a Rust con `invoke`.
  - **El token nunca llega a JavaScript.**
  - La API sigue rechazando las peticiones con `Origin` (ADR 0015): no hace falta abrirle una excepción.
- **Política de contenido estricta** (CSP): la interfaz solo carga lo suyo (`'self'`) y solo habla con Rust (`ipc:`).
- **Las fuentes van dentro de la app** (Inter y JetBrains Mono de `@fontsource-variable`, licencia OFL): nunca se
  piden a Google ni a ningún otro sitio.
- La app busca `locall.exe` a su lado (o en `LOCALL_EXE`) y lo arranca en segundo plano si no está abierto.
- La app **no está en los `default-members`** del espacio de trabajo: `cargo build` y `cargo test` no la compilan
  (Tauri son más de 200 crates y necesita la interfaz construida). Para la app: `npm run tauri dev` en
  `crates/locall-app`.

## Motivos

- **Tauri frente a una interfaz nativa en Rust** (egui, iced): las maquetas de la investigación son HTML/CSS y se
  reutilizan; es más fácil de aprender; Jan, un proyecto parecido, lo usa con éxito.
- **Tauri frente a Electron:** Electron incluye un navegador entero (~100 MB por app).
- **El token en Rust:** si algún contenido malicioso llegara a ejecutarse en la interfaz (por ejemplo, el texto de un
  modelo mal mostrado), no podría robar el token ni usar la API por su cuenta.

## Consecuencias

- **Dependencias:** Tauri compila 233 crates en Windows (casi todas MIT/Apache; **5 son MPL-2.0**, que obligan a
  conservar su licencia y citarlas al distribuir). La interfaz instala 29 paquetes de npm (93 MB en `node_modules`,
  fuera de Git; 0 vulnerabilidades conocidas al instalar). Todas las licencias son MIT, Apache-2.0 u OFL-1.1.
- Hace falta Node.js para construir la interfaz (no para usar la app).
- El icono actual es **provisional** (un "L." dibujado por nosotros) hasta tener el definitivo.
- Probar la interfaz automáticamente es más difícil que probar el núcleo: la parte Rust se prueba como el resto, y
  las pantallas se comprueban viéndolas.
