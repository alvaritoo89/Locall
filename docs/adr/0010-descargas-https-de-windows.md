# ADR 0010 · Descargas con `ureq` y el HTTPS de Windows

- **Estado:** aceptada
- **Fecha:** 2026-11-08

## Decisión

- El descargador (`locall-download`) usa **`ureq` 3** (cliente HTTP bloqueante, sin `async`), con
  `default-features = false` y solo la función `native-tls-no-default`.
- El HTTPS lo hace **el propio Windows** (SChannel, mediante `native-tls`) con **sus certificados**
  (`RootCerts::PlatformVerifier`). Locall no lleva su propia pila TLS ni su propia lista de certificados.
- Toda descarga lleva **huella sha256 obligatoria**, calculada mientras se descarga. Si no coincide, se descarta.
- **Reanudable:** lo descargado se guarda en `tmp\sha256-<hex>.parcial` y se continúa con `Range`. La limpieza de
  temporales no toca los `.parcial`. Si el servidor ignora el `Range` (responde 200) o el rango no encaja, se
  empieza de cero; nunca se mezclan datos.
- **Atascos:** si no llega ningún byte en 60 s, se corta y se reanuda. **Reintentos** con espera creciente
  (1 s → 30 s). El contador se reinicia cuando llegan datos. Ante un `429` se espera lo que pide `Retry-After`
  (máximo 120 s). Los errores 4xx (salvo 416 y 429) no se reintentan.
- Se comprueba el **espacio libre** antes de empezar. El progreso se cuenta con los bytes escritos.
- En v1 **no se usa token** de Hugging Face (solo modelos públicos).

## Motivos

- **Menos dependencias** (regla 3): con `native-tls` no se compila ninguna librería criptográfica. En la versión con
  rustls habría que añadir `rustls`, `ring` y `webpki-roots`, que además exige compilar C/ensamblador.
- **Certificados del sistema:** si una empresa usa un proxy con su propio certificado instalado en Windows, Locall
  funciona sin configurar nada. Las actualizaciones de certificados llegan con Windows Update.
- **Bloqueante es suficiente:** las descargas son pocas y largas; un hilo por descarga es simple de entender y
  probar. `async` (tokio + reqwest) añadiría decenas de dependencias sin ganar nada aquí.
- Verificado en esta máquina: descarga real de Hugging Face, incluida la redirección a su CDN y la reanudación con
  `Range` tras una pausa.

## Alternativas descartadas

- **`reqwest` + tokio:** potente, pero mucho más grande; lo reconsideraremos si la API (Fase 3) ya trae tokio.
- **`ureq` con rustls:** independiente del sistema, pero más dependencias y certificados propios que mantener.
- **Llamar a `curl.exe`:** sin control fino del progreso ni de los errores; depende de un programa externo.

## Consecuencias

- Dependencias nuevas (todas MIT o MIT/Apache): `ureq`, `ureq-proto`, `http`, `httparse`, `native-tls`, `schannel`,
  `rustls-pki-types`, `base64`, `bytes`, `log`, `percent-encoding`, `utf8-zero`, `der`, `pem-rfc7468`, `const-oid`,
  `base64ct`, `zeroize`, `itoa`.
- **A revisar al llevar Locall a Linux:** allí `native-tls` usa OpenSSL (habría que tenerlo instalado o compilarlo).
  Entonces decidiremos entre OpenSSL y rustls.
- El descargador es bloqueante: la app y la API deben llamarlo desde un hilo propio.
