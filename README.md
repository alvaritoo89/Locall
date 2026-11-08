# Locall.

Plataforma gratuita y de código abierto para descargar, gestionar y ejecutar modelos de IA **en tu propio
ordenador**. Sin cuentas, sin suscripciones y sin enviar tus conversaciones a ningún sitio.

> Estado: en desarrollo temprano (Fase 2: núcleo). Todavía no hay nada que instalar.

## Principios

- **Local primero:** los modelos se ejecutan en tu equipo; la privacidad es un principio de diseño.
- **Gratis de verdad:** sin servicios de pago obligatorios.
- **Honesto:** no se promete compatibilidad ni rendimiento sin haberlo medido.
- **Para cualquier hardware:** pensado también para equipos sin gráfica dedicada.

## Arquitectura (resumen)

Un núcleo en Rust gestiona los modelos y controla el motor de inferencia
([llama.cpp](https://github.com/ggml-org/llama.cpp), binarios oficiales) como un proceso aparte. La API local,
la línea de comandos y la aplicación de escritorio son clientes de ese núcleo.

Las decisiones técnicas y sus motivos están en [`docs/adr/`](docs/adr/).

## Licencia

[Apache-2.0](LICENSE).
