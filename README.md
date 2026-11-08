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
> Locall está en **desarrollo temprano** (Fase 3: la API local). Todavía no hay nada que instalar.

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

## Medido, no prometido

Pruebas reales en un portátil normal (Intel Core i5-1334U, gráfica integrada Iris Xe, 16 GB de RAM):

| Modelo | Escribir (CPU) | Leer el prompt (gráfica integrada) | Memoria (sin mapeo) |
|---|---|---|---|
| Qwen2.5 1.5B · Q4_K_M | ~33 tokens/s | ~509 tokens/s | ~1,2 GB |
| Qwen3 4B · Q4_K_M | ~12 tokens/s | ~189 tokens/s | ~2,5 GB |
| Qwen2.5 7B · Q4_K_M | ~7,6 tokens/s | ~115 tokens/s | ~4,6 GB |

## Estado

- [x] Fase 0: investigación y pruebas de concepto en hardware real
- [x] Fase 1: control del motor desde Rust (arranque, streaming, parada, protección ante cierres bruscos)
- [x] Fase 2: núcleo (lector GGUF, almacén, descargas verificadas, motor, planificador, configuración y registros privados)
- [ ] **Fase 3: API local** · ✓ núcleo · ✓ servidor y seguridad · conversación · descargas · compatibilidad con OpenAI
- [ ] Fase 4: línea de comandos (`locall`)
- [ ] Fase 5: aplicación de escritorio

## Estructura del repositorio

| Ruta | Contenido |
|---|---|
| [`crates/locall-gguf`](crates/locall-gguf) | Lector de cabeceras GGUF con límites de seguridad |
| [`crates/locall-store`](crates/locall-store) | Almacén de modelos: huellas sha256, snapshots con enlaces duros, espacio y limpieza |
| [`crates/locall-system`](crates/locall-system) | Información del equipo (memoria, procesador, hilos) y Job Object de Windows |
| [`crates/locall-compat`](crates/locall-compat) | ¿Cabe este modelo y lo entiende el motor? Estimación de memoria y compatibilidad |
| [`crates/locall-registry`](crates/locall-registry) | Registro de modelos instalados (SQLite con migraciones) |
| [`crates/locall-download`](crates/locall-download) | Descargas verificadas por sha256, reanudables y con detección de atascos |
| [`crates/locall-engine`](crates/locall-engine) | Arranca, vigila y para `llama-server` de forma segura y conversa con él por trozos (con cancelación) |
| [`crates/locall-scheduler`](crates/locall-scheduler) | Planificador: un modelo a la vez, comprobación de memoria antes de cargar, cola por orden de llegada y descarga por inactividad |
| [`crates/locall-config`](crates/locall-config) | Carpeta de datos, ajustes del usuario (`config.toml`) y registros diarios sin datos de las conversaciones |
| [`crates/locall-core`](crates/locall-core) | Núcleo: instalar de Hugging Face (fijado al commit y verificado), importar, listar, eliminar y conversar |
| [`crates/locall-api`](crates/locall-api) | API local (axum): solo 127.0.0.1, token y protección frente a webs (`Host`, `Origin`) |
| [`crates/locall`](crates/locall) | El programa `locall` (`locall serve`; la línea de comandos llegará en la Fase 4) |
| [`docs/adr`](docs/adr) | Decisiones de arquitectura y sus motivos |

## Desarrollo

Requisitos: Windows 10/11, [Rust](https://www.rust-lang.org/) 1.99 o superior y Visual Studio Build Tools (C++).

```bash
cargo test
```

## Licencia

[Apache-2.0](LICENSE). Los modelos de IA tienen sus propias licencias, que Locall muestra antes de descargarlos.
