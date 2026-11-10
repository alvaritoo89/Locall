// Explorar: buscar modelos en Hugging Face por nombre e instalarlos.
//
// Solo se envía a Hugging Face el texto que escribes (ni cuenta ni datos del equipo). Locall no tiene un catálogo
// propio todavía (llegará en la Fase 6): aquí se ve lo que hay en Hugging Face, ordenado por descargas.

import { type FormEvent, useRef, useState } from "react";

import { ArchivosDelRepo, type Instalaciones } from "./Instalar";
import { type Encontrado, type Repositorio, buscarModelos, cifraCorta, consultarRepo, textoError } from "./locall";

/** Para qué sirve un modelo, en palabras. (Solo las tareas más habituales; las demás se muestran tal cual.) */
const TAREAS: Record<string, string> = {
  "text-generation": "texto",
  "image-text-to-text": "texto e imágenes",
  "feature-extraction": "incrustaciones (no conversa)",
  "sentence-similarity": "incrustaciones (no conversa)",
};

export function Explorar({ instalaciones }: { instalaciones: Instalaciones }) {
  const [texto, setTexto] = useState("");
  const [buscando, setBuscando] = useState(false);
  const [resultados, setResultados] = useState<Encontrado[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [abierto, setAbierto] = useState<string | null>(null);
  const [repo, setRepo] = useState<Repositorio | null>(null);
  const [errorRepo, setErrorRepo] = useState<string | null>(null);
  /** El último repositorio pedido (las respuestas de otros pedidos antes se ignoran). */
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
      <h1>Explorar</h1>
      <section className="tarjeta ancha" aria-labelledby="titulo-buscar">
        <h2 id="titulo-buscar">Buscar en Hugging Face</h2>
        <p className="pequeño tenue">
          Busca por nombre (por ejemplo, «qwen3», «llama» o «gemma»). Solo aparecen repositorios con modelos GGUF, los
          más descargados primero. Lo que escribes se envía a Hugging Face; nada más.
        </p>
        <form className="buscador" onSubmit={buscar} role="search">
          <label htmlFor="buscar" className="solo-lectores">Buscar modelos</label>
          <input id="buscar" className="campo" placeholder="qwen3" value={texto} maxLength={100}
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
