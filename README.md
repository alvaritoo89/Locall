<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/logo-oscuro.png">
    <img src="docs/assets/logo-claro.png" alt="Locall." width="300">
  </picture>
</p>

<p align="center"><b>Modelos de IA en tu propio ordenador.</b><br>Gratis, local y privado.</p>

<p align="center">
  <img src="docs/assets/insignia-estado.svg" alt="estado: en desarrollo">
  <img src="docs/assets/insignia-licencia.svg" alt="licencia: Apache-2.0">
  <img src="docs/assets/insignia-rust.svg" alt="rust: 1.99+">
  <img src="docs/assets/insignia-plataforma.svg" alt="plataforma: Windows">
  <img src="docs/assets/insignia-privacidad.svg" alt="privacidad: sin telemetría">
</p>

---

**Locall** es una plataforma gratuita y de código abierto para descargar, gestionar y ejecutar modelos de
inteligencia artificial **en tu propio equipo**. Sin cuentas, sin suscripciones y sin enviar tus conversaciones a
ningún sitio.

> [!NOTE]
> Locall está en **desarrollo temprano**. Ya se puede usar desde la línea de comandos compilándolo desde el código
> (ver [Probarlo](#probarlo)); el instalador llegará con la aplicación de escritorio.

## Principios

| | |
|---|---|
| **Local primero** | Los modelos se ejecutan en tu ordenador. La privacidad es un principio de diseño, no una opción. |
| **Gratis de verdad** | Sin servicios de pago obligatorios ni servidores que pagar por cada usuario. |
| **Honesto** | Nada de prometer compatibilidad o velocidad sin haberlo medido. |
| **Para cualquier equipo** | Pensado también para portátiles sin gráfica dedicada. |

## Cómo funciona

```
   CLI  ·  App de escritorio  ·  Tus aplicaciones
                    │  API local (solo en tu equipo)
                    ▼
         Núcleo de Locall (Rust)
         modelos · descargas verificadas · memoria
                    │  proceso aparte, protegido
                    ▼
          llama.cpp (binarios oficiales)
```

Un núcleo en Rust gestiona los modelos y controla el motor de inferencia
[llama.cpp](https://github.com/ggml-org/llama.cpp) como un proceso aparte. La línea de comandos, la aplicación de
escritorio y la API local son clientes de ese núcleo.

## Probarlo

Todavía no hay instalador. Mientras tanto, desde el código fuente (Windows 10/11):

1. Instala [Rust](https://www.rust-lang.org/) 1.99 o superior y Visual Studio Build Tools (C++).
2. Compila Locall:

   ```bash
   cargo build --release
   ```

3. Descarga el motor: `llama-b12189-bin-win-cpu-x64.zip` de la
   [versión b12189 de llama.cpp](https://github.com/ggml-org/llama.cpp/releases/tag/b12189)
   (sha256 `f912ea5e3e397c63405c81e732131ab0fa6e72411c80546bb5ad643f7dd82aac`). Descomprímelo en
   `target\release\motor\`, de forma que quede `target\release\motor\llama-server.exe`.
4. Úsalo desde `target\release` (en PowerShell, los programas de la carpeta actual se escriben con `.\` delante):

   ```text
   .\locall catalog                                         modelos recomendados (compatibles)
   .\locall pull qwen3-4b                                   instalar uno del catálogo
   .\locall pull Qwen/Qwen3-4B-GGUF                         ¿qué versiones tiene?
   .\locall pull Qwen/Qwen3-4B-GGUF Qwen3-4B-Q4_K_M.gguf    instalarlo (2,5 GB)
   .\locall run qwen3-4b-q4_k_m                             conversar (Ctrl+C corta la respuesta)
   .\locall list                                            tus modelos
   .\locall help                                            todas las órdenes
   ```

Locall se abre solo en segundo plano cuando hace falta y se cierra solo tras un rato sin usarse. Sus datos (modelos,
ajustes, conversaciones guardadas, registros) están en `%LOCALAPPDATA%\Locall` y no salen de tu equipo. Otros programas pueden usarlo con la
[API local](docs/adr/0015-nucleo-y-api-local.md), también en formato compatible con OpenAI
([ADR 0016](docs/adr/0016-compatibilidad-openai-y-cierre.md)).

## Medido, no prometido

Pruebas reales en un portátil normal (procesador de portátil reciente, gráfica integrada y 16 GB de RAM). En otros equipos
las cifras serán distintas:

| Modelo | Escribir (CPU) | Leer el prompt (gráfica integrada) | Memoria (sin mapeo) |
|---|---|---|---|
| Qwen2.5 1.5B · Q4_K_M | ~33 tokens/s | ~509 tokens/s | ~1,2 GB |
| Qwen3 4B · Q4_K_M | ~12 tokens/s | ~189 tokens/s | ~2,5 GB |
| Qwen2.5 7B · Q4_K_M | ~7,6 tokens/s | ~115 tokens/s | ~4,6 GB |

## Estado

- [x] Fase 0: investigación y pruebas de concepto en hardware real
- [x] Fase 1: control del motor desde Rust (arranque, streaming, parada, protección ante cierres bruscos)
- [x] Fase 2: núcleo (lector GGUF, almacén, descargas verificadas, motor, planificador, configuración y registros privados)
- [x] Fase 3: API local (núcleo, servidor con token, conversación, descargas, formato compatible con OpenAI, cierre por inactividad)
- [x] Fase 4: línea de comandos (`locall pull`, `run`, `list`, `show`, `rm`, `import`, `status`, `stop`) · **MVP**
- [x] Fase 5: aplicación de escritorio (inicio, biblioteca, chat con conversaciones guardadas y borrado real, explorar Hugging Face, ajustes, importar, atajos de teclado, accesibilidad, modo oscuro)
- [ ] Fase 6: catálogo de modelos compatibles y web de descarga
- [ ] Fase 7: endurecimiento (seguridad y fiabilidad)
- [ ] Fase 8: instalador, firma y distribución

## Estructura del repositorio

| Ruta | Contenido |
|---|---|
| [`crates/locall-gguf`](crates/locall-gguf) | Lector de cabeceras GGUF con límites de seguridad |
| [`crates/locall-store`](crates/locall-store) | Almacén de modelos: huellas sha256, snapshots con enlaces duros, espacio y limpieza |
| [`crates/locall-system`](crates/locall-system) | Información del equipo (memoria, procesador, hilos) y Job Object de Windows |
| [`crates/locall-compat`](crates/locall-compat) | ¿Cabe este modelo y lo entiende el motor? Estimación de memoria y compatibilidad |
| [`crates/locall-registry`](crates/locall-registry) | Registro de modelos instalados y conversaciones guardadas (SQLite con migraciones y borrado real) |
| [`crates/locall-download`](crates/locall-download) | Descargas verificadas por sha256, reanudables y con detección de atascos |
| [`crates/locall-engine`](crates/locall-engine) | Arranca, vigila y para `llama-server` de forma segura y conversa con él por trozos (con cancelación) |
| [`crates/locall-scheduler`](crates/locall-scheduler) | Planificador: un modelo a la vez, comprobación de memoria antes de cargar, cola por orden de llegada y descarga por inactividad |
| [`crates/locall-config`](crates/locall-config) | Carpeta de datos, ajustes del usuario (`config.toml`) y registros diarios sin datos de las conversaciones |
| [`crates/locall-catalogo`](crates/locall-catalogo) | Catálogo de modelos compatibles: formato, validación y firma minisign |
| [`crates/locall-core`](crates/locall-core) | Núcleo: instalar de Hugging Face (fijado al commit y verificado), importar, listar, eliminar, conversar y guardar las conversaciones |
| [`crates/locall-api`](crates/locall-api) | API local (axum): solo 127.0.0.1, token y protección frente a webs (`Host`, `Origin`) |
| [`crates/locall-cli`](crates/locall-cli) | Línea de comandos: cliente de la API que abre Locall en segundo plano cuando hace falta |
| [`crates/locall-app`](crates/locall-app) | Aplicación de escritorio (Tauri 2 + React): un cliente más de la API, con el token fuera de la interfaz |
| [`crates/locall`](crates/locall) | El programa `locall`: `locall serve` y las órdenes de la línea de comandos |
| [`docs/adr`](docs/adr) | Decisiones de arquitectura y sus motivos |

## Desarrollo

Requisitos: los de [Probarlo](#probarlo). Algunas pruebas usan el motor y modelos reales y se saltan solas si no los
encuentran (rutas por defecto en `C:\Locall-poc`); las que descargan de internet están marcadas `#[ignore]`.

```bash
cargo test
```

La aplicación de escritorio (`crates/locall-app`) necesita además [Node.js](https://nodejs.org/) y se abre en modo desarrollo con `npm install` y `npm run tauri dev` dentro de esa carpeta.

## Licencia

[Apache-2.0](LICENSE). Los modelos de IA tienen sus propias licencias: Locall las muestra al consultar un repositorio
(`locall pull <repositorio>`) y en los detalles de cada modelo (`locall show`).
