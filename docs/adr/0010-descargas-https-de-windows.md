# ADR 0010 · Downloads with `ureq` and Windows HTTPS

- **Status:** accepted
- **Date:** 2026-11-08

## Decision

- The downloader (`locall-download`) uses **`ureq` 3** (a blocking HTTP client, no `async`), with
  `default-features = false` and only the `native-tls-no-default` feature.
- HTTPS is handled by **Windows itself** (SChannel, through `native-tls`) with **its certificates**
  (`RootCerts::PlatformVerifier`). Locall ships neither its own TLS stack nor its own certificate list.
- Every download carries a **mandatory sha256 fingerprint**, computed while downloading. If it does not match, the file
  is discarded.
- **Resumable:** downloaded data is kept in `tmp\sha256-<hex>.parcial` and continued with `Range`. Temporary-file
  cleanup does not touch `.parcial` files. If the server ignores `Range` (responds 200) or the range does not fit, the
  download starts from scratch; data is never mixed.
- **Stalls:** if no byte arrives for 60 s, the connection is cut and resumed. **Retries** with growing waits
  (1 s → 30 s). The counter resets when data arrives. On a `429`, Locall waits as long as `Retry-After` asks (at most
  120 s). 4xx errors (except 416 and 429) are not retried.
- **Free space** is checked before starting. Progress is counted in bytes written.
- v1 uses **no** Hugging Face token (public models only).

## Rationale

- **Fewer dependencies** (rule 3): with `native-tls` no cryptographic library is compiled. The rustls variant would add
  `rustls`, `ring` and `webpki-roots`, which also requires compiling C/assembly.
- **System certificates:** if a company uses a proxy with its own certificate installed in Windows, Locall works with
  no configuration. Certificate updates arrive through Windows Update.
- **Blocking is enough:** downloads are few and long; one thread per download is simple to understand and to test.
  `async` (tokio + reqwest) would add dozens of dependencies for no gain here.
- Verified on this machine: a real download from Hugging Face, including the redirect to its CDN and resuming with
  `Range` after a pause.

## Rejected alternatives

- **`reqwest` + tokio:** powerful, but much bigger; to be reconsidered if the API (Phase 3) already brings tokio.
- **`ureq` with rustls:** independent of the system, but more dependencies and our own certificates to maintain.
- **Calling `curl.exe`:** no fine control over progress or errors; depends on an external program.

## Consequences

- New dependencies (all MIT or MIT/Apache): `ureq`, `ureq-proto`, `http`, `httparse`, `native-tls`, `schannel`,
  `rustls-pki-types`, `base64`, `bytes`, `log`, `percent-encoding`, `utf8-zero`, `der`, `pem-rfc7468`, `const-oid`,
  `base64ct`, `zeroize`, `itoa`.
- **To review when porting Locall to Linux:** there `native-tls` uses OpenSSL (it would have to be installed or
  compiled). We will then decide between OpenSSL and rustls.
- The downloader is blocking: the app and the API must call it from their own thread.
