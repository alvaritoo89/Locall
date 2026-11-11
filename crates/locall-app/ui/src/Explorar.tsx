// Explorar: search Hugging Face models by name and install them.
//
// Only the text you type is sent to Hugging Face (no account, no computer data). Locall's own catalog is shown first
// («Recomendados»); the search shows what is on Hugging Face, sorted by downloads.

import { type FormEvent, useEffect, useRef, useState } from "react";

import { ArchivosDelRepo, type Instalaciones, clave } from "./Instalar";
import {
  type Catalogo,
  type Encontrado,
  type Repositorio,
  buscarModelos,
  cancelarInstalacion,
  cifraCorta,
  consultarRepo,
  pedirCatalogo,
  tamano,
  textoEncaje,
  textoError,
} from "./locall";

/** What a model is for, in words. (Only the most common tasks; the rest are shown as is.) */
const TAREAS: Record<string, string> = {
  "text-generation": "texto",
  "image-text-to-text": "texto e imágenes",
  "feature-extraction": "incrustaciones (no conversa)",
  "sentence-similarity": "incrustaciones (no conversa)",
};

interface Props {
  instalaciones: Instalaciones;
  /** Whether the screen is in view (the catalog is requested the first time it is seen, not when the app opens). */
  visible: boolean;
  /** Changes when the installed models change (to mark «Instalado»). */
  modelosInstalados: number;
}

export function Explorar({ instalaciones, visible, modelosInstalados }: Props) {
  return (
    <>
      <h1>Explorar</h1>
      <Recomendados instalaciones={instalaciones} visible={visible} modelosInstalados={modelosInstalados} />
      <BuscarEnHuggingFace instalaciones={instalaciones} />
    </>
  );
}

// ─── Recommended by Locall (the catalog) ──────────────────────────────────────

function Recomendados({ instalaciones, visible, modelosInstalados }: Props) {
  const [catalogo, setCatalogo] = useState<Catalogo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const pedido = useRef(false);

  useEffect(() => {
    if (!visible) return;
    if (pedido.current && catalogo) {
      // We already have it: only refresh «Instalado» (without checking GitHub again: Locall does that once a day).
      pedirCatalogo().then(setCatalogo).catch(() => {});
      return;
    }
    pedido.current = true;
    pedirCatalogo()
      .then((c) => (setCatalogo(c), setError(null)))
      .catch((e) => setError(textoError(e)));
  }, [visible, modelosInstalados]);

  return (
    <section className="tarjeta ancha" aria-labelledby="titulo-recomendados">
      <div className="titulo-tarjeta">
        <h2 id="titulo-recomendados">Recomendados por Locall</h2>
        {catalogo && <span className="pequeño tenue">{catalogo.models.length} modelos</span>}
      </div>
      <p className="pequeño tenue">
        Todos son <strong>compatibles</strong>: el motor de Locall reconoce su formato. No se han probado uno a uno.
        «¿Cabe?» es aproximado, con la memoria libre ahora (cerrar otros programas deja más).
      </p>
      {error && <p className="pequeño tono-error" role="alert">{error}</p>}
      {catalogo?.warning && <p className="pequeño tono-aviso">{catalogo.warning}</p>}
      {!catalogo && !error && <p className="pequeño tenue">Cargando el catálogo…</p>}
      {catalogo && (
        <ul className="catalogo">
          {catalogo.models.map((m) => {
            const i = instalaciones.estado[clave("catalogo", m.name)];
            const enCurso = i && !i.error && !i.hecho;
            const pct =
              i?.progreso && i.progreso.total_bytes > 0
                ? Math.floor((i.progreso.downloaded_bytes / i.progreso.total_bytes) * 100)
                : 0;
            const encaje = textoEncaje(m.fit_estimate);
            const instalado = m.installed || !!i?.hecho;
            return (
              <li key={m.name} className="ficha">
                <div className="ficha-cabecera">
                  <h3 className="ficha-titulo">{m.title}</h3>
                  <span className="etiqueta-compatible" title="El motor reconoce su formato">compatible</span>
                </div>
                <p className="pequeño tenue">
                  {m.publisher} · {tamano(m.size_bytes)} · licencia {m.license}
                </p>
                <p className="pequeño">{m.description}</p>
                <p className={`pequeño tono-${encaje.tono}`}>¿Cabe? {encaje.texto} (aprox.)</p>
                <div className="ficha-pie">
                  <code className="cli">locall pull {m.name}</code>
                  <span className="hueco" />
                  {instalado ? (
                    <span className="cargado">Instalado</span>
                  ) : enCurso ? (
                    <button type="button" className="boton" onClick={() => cancelarInstalacion("catalogo", m.name)}>
                      Cancelar
                    </button>
                  ) : (
                    <button type="button" className="boton" onClick={() => instalaciones.instalarDelCatalogo(m.name)}
                      aria-label={`Instalar ${m.title}`}>
                      {i?.error ? "Reintentar" : "Instalar"}
                    </button>
                  )}
                </div>
                {enCurso && (
                  <div className="progreso">
                    <div className="progreso-barra" role="progressbar" aria-valuemin={0} aria-valuemax={100}
                      aria-valuenow={pct} aria-label={`Descargando ${m.title}`}>
                      <div style={{ width: `${pct}%` }} />
                    </div>
                    <span className="mono pequeño">
                      {i?.progreso
                        ? `${pct} % · ${tamano(i.progreso.downloaded_bytes)} de ${tamano(i.progreso.total_bytes)}`
                        : "Comprobando contra el catálogo firmado…"}
                    </span>
                  </div>
                )}
                {i?.error && <p className="pequeño tono-error" role="alert">{i.error}</p>}
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

// ─── Search Hugging Face ──────────────────────────────────────────────────────

function BuscarEnHuggingFace({ instalaciones }: { instalaciones: Instalaciones }) {
  const [texto, setTexto] = useState("");
  const [buscando, setBuscando] = useState(false);
  const [resultados, setResultados] = useState<Encontrado[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [abierto, setAbierto] = useState<string | null>(null);
  const [repo, setRepo] = useState<Repositorio | null>(null);
  const [errorRepo, setErrorRepo] = useState<string | null>(null);
  /** The last repository requested (responses to earlier requests are ignored). */
  const pedido = useRef<string | null>(null);

  const buscar = async (e: FormEvent) => {
    e.preventDefault();
    if (!texto.trim() || buscando) return;
    setBuscando(true);
    setError(null);
    setAbierto(null);
    try {
      setResultados(await buscarModelos(texto.trim()));
    } catch (e) {
      setError(textoError(e));
      setResultados(null);
    } finally {
      setBuscando(false);
    }
  };

  const abrir = async (nombre: string) => {
    if (abierto === nombre) {
      setAbierto(null);
      pedido.current = null;
      return;
    }
    setAbierto(nombre);
    pedido.current = nombre;
    setRepo(null);
    setErrorRepo(null);
    try {
      const r = await consultarRepo(nombre);
      if (pedido.current === nombre) setRepo(r);
    } catch (e) {
      if (pedido.current === nombre) setErrorRepo(textoError(e));
    }
  };

  return (
    <>
      <section className="tarjeta ancha" aria-labelledby="titulo-buscar">
        <h2 id="titulo-buscar">Buscar en Hugging Face</h2>
        <p className="pequeño tenue">
          Busca por nombre (por ejemplo, «qwen3», «llama» o «gemma»). Solo aparecen repositorios con modelos GGUF, los
          más descargados primero. Lo que escribes se envía a Hugging Face; nada más.
        </p>
        <form className="buscador" onSubmit={buscar} role="search">
          <label htmlFor="buscar" className="solo-lectores">Buscar modelos</label>
          <input id="buscar" className="campo" placeholder="qwen3   (Ctrl+K)" value={texto} maxLength={100}
            aria-keyshortcuts="Control+K"
            onChange={(e) => setTexto(e.target.value)} autoComplete="off" spellCheck={false} />
          <button type="submit" className="boton primario" disabled={buscando || !texto.trim()}>
            {buscando ? "Buscando…" : "Buscar"}
          </button>
        </form>
        {error && <p className="pequeño tono-error" role="alert">{error}</p>}
        {resultados?.length === 0 && <p className="pequeño tenue">No hay repositorios con GGUF con ese nombre.</p>}
      </section>

      {resultados && resultados.length > 0 && (
        <section className="tarjeta ancha sin-relleno" aria-label="Resultados">
          <ul className="resultados">
            {resultados.map((r) => {
              const esAbierto = abierto === r.repo;
              return (
                <li key={r.repo} className={`resultado${esAbierto ? " abierto" : ""}`}>
                  <button type="button" className="resultado-fila" aria-expanded={esAbierto}
                    onClick={() => abrir(r.repo)}>
                    <span className="resultado-nombre mono">{r.repo}</span>
                    <span className="pequeño tenue">
                      {r.task ? (TAREAS[r.task] ?? r.task) : "—"}
                      {" · "}licencia {r.license ?? "sin indicar"}
                    </span>
                    <span className="hueco" />
                    <span className="mono pequeño tenue" title={`${r.downloads} descargas`}>
                      ↓ {cifraCorta(r.downloads)}
                    </span>
                    <span className="mono pequeño tenue" title={`${r.likes} «me gusta»`}>♥ {cifraCorta(r.likes)}</span>
                    <span aria-hidden="true" className="tenue">{esAbierto ? "▾" : "▸"}</span>
                  </button>
                  {esAbierto && (
                    <div className="resultado-detalle">
                      {!repo && !errorRepo && <p className="pequeño tenue">Consultando el repositorio…</p>}
                      {errorRepo && <p className="pequeño tono-error" role="alert">{errorRepo}</p>}
                      {repo?.repo === r.repo && <ArchivosDelRepo repo={repo} instalaciones={instalaciones} />}
                    </div>
                  )}
                </li>
              );
            })}
          </ul>
        </section>
      )}
    </>
  );
}
