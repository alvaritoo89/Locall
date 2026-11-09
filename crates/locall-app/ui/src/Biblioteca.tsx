// Biblioteca: los modelos instalados (con eliminar) y añadir uno desde Hugging Face.

import { type FormEvent, useState } from "react";

import { EstadoDelModelo, NombreDelModelo } from "./comun";
import {
  type EstadoLocall,
  type Modelo,
  type Progreso,
  type Repositorio,
  cancelarInstalacion,
  consultarRepo,
  eliminarModelo,
  instalarModelo,
  tamano,
  textoError,
} from "./locall";

interface Props {
  estado: EstadoLocall | null;
  modelos: Modelo[] | null;
  recargar: () => void;
}

export function Biblioteca({ estado, modelos, recargar }: Props) {
  return (
    <>
      <h1>Biblioteca</h1>
      <Instalados estado={estado} modelos={modelos} recargar={recargar} />
      <AnadirDesdeHuggingFace recargar={recargar} />
    </>
  );
}

// ─── Instalados ───────────────────────────────────────────────────────────────

function Instalados({ estado, modelos, recargar }: Props) {
  const [confirmando, setConfirmando] = useState<string | null>(null);
  const [borrando, setBorrando] = useState<string | null>(null);
  const [mensaje, setMensaje] = useState<{ texto: string; error: boolean } | null>(null);

  const eliminar = async (m: Modelo) => {
    setBorrando(m.id);
    setMensaje(null);
    try {
      const liberados = await eliminarModelo(m.id);
      setMensaje({ texto: `Eliminado «${m.id}»: ${tamano(liberados)} liberados.`, error: false });
      recargar();
    } catch (e) {
      setMensaje({ texto: textoError(e), error: true });
    } finally {
      setBorrando(null);
      setConfirmando(null);
    }
  };

  return (
    <section className="tarjeta ancha sin-relleno" aria-labelledby="titulo-instalados">
      <div className="cabecera-tabla">
        <h2 id="titulo-instalados">Instalados</h2>
        {modelos && <span className="tenue pequeño">{modelos.length} modelos</span>}
      </div>
      {mensaje && (
        <p className={`relleno pequeño ${mensaje.error ? "tono-error" : ""}`} role={mensaje.error ? "alert" : "status"}>
          {mensaje.texto}
        </p>
      )}
      {modelos === null && <p className="tenue relleno">Cargando la lista…</p>}
      {modelos?.length === 0 && <p className="tenue relleno">Todavía no tienes modelos. Añade uno abajo.</p>}
      {modelos && modelos.length > 0 && (
        <table className="tabla">
          <thead>
            <tr>
              <th scope="col">Modelo</th>
              <th scope="col">Tamaño</th>
              <th scope="col">Origen</th>
              <th scope="col">Licencia</th>
              <th scope="col">Estado</th>
              <th scope="col"><span className="solo-lectores">Acciones</span></th>
            </tr>
          </thead>
          <tbody>
            {modelos.map((m) => (
              <tr key={m.id}>
                <td><NombreDelModelo modelo={m} /></td>
                <td className="mono">{tamano(m.size_bytes)}</td>
                <td className="pequeño">{m.source.replace(/^hf:/, "Hugging Face · ")}</td>
                <td>{m.license ?? "—"}</td>
                <td><EstadoDelModelo modelo={m} estado={estado} /></td>
                <td className="acciones">
                  {confirmando === m.id ? (
                    <span className="confirmar">
                      <span className="pequeño">¿Eliminar {tamano(m.size_bytes)}?</span>
                      <button type="button" className="boton peligro" disabled={borrando === m.id}
                        onClick={() => eliminar(m)}>
                        {borrando === m.id ? "Eliminando…" : "Eliminar"}
                      </button>
                      <button type="button" className="boton" onClick={() => setConfirmando(null)}>
                        Cancelar
                      </button>
                    </span>
                  ) : (
                    <button type="button" className="boton" aria-label={`Eliminar ${m.id}`}
                      onClick={() => setConfirmando(m.id)}>
                      Eliminar
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}

// ─── Añadir desde Hugging Face ────────────────────────────────────────────────

interface Instalando {
  progreso: Progreso | null;
  error: string | null;
  hecho: string | null;
}

function AnadirDesdeHuggingFace({ recargar }: { recargar: () => void }) {
  const [texto, setTexto] = useState("");
  const [buscando, setBuscando] = useState(false);
  const [repo, setRepo] = useState<Repositorio | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [instalando, setInstalando] = useState<Record<string, Instalando>>({});

  const buscar = async (e: FormEvent) => {
    e.preventDefault();
    if (!texto.trim()) return;
    setBuscando(true);
    setError(null);
    setRepo(null);
    try {
      setRepo(await consultarRepo(texto.trim()));
    } catch (e) {
      setError(textoError(e));
    } finally {
      setBuscando(false);
    }
  };

  const instalar = async (repositorio: string, archivo: string) => {
    const poner = (cambio: Partial<Instalando>) =>
      setInstalando((actual) => ({
        ...actual,
        [archivo]: { ...(actual[archivo] ?? { progreso: null, error: null, hecho: null }), ...cambio },
      }));
    poner({ progreso: null, error: null, hecho: null });
    try {
      const modelo = await instalarModelo(repositorio, archivo, (p) => poner({ progreso: p }));
      poner({ hecho: `Instalado «${modelo.id}» (${tamano(modelo.size_bytes)}, verificado con sha256).` });
      recargar();
    } catch (e) {
      poner({ error: textoError(e) });
    }
  };

  return (
    <section className="tarjeta ancha" aria-labelledby="titulo-anadir">
      <h2 id="titulo-anadir">Añadir desde Hugging Face</h2>
      <p className="pequeño tenue">
        Escribe el repositorio (propietario/nombre). Locall fija la versión exacta, comprueba que el motor puede usar el
        modelo antes de descargarlo y verifica cada archivo con sha256.
      </p>
      <form className="buscador" onSubmit={buscar}>
        <label htmlFor="repo" className="solo-lectores">Repositorio de Hugging Face</label>
        <input id="repo" className="campo mono" placeholder="Qwen/Qwen3-4B-GGUF" value={texto}
          onChange={(e) => setTexto(e.target.value)} autoComplete="off" spellCheck={false} />
        <button type="submit" className="boton primario" disabled={buscando || !texto.trim()}>
          {buscando ? "Buscando…" : "Buscar"}
        </button>
      </form>
      {error && <p className="pequeño tono-error" role="alert">{error}</p>}
      {repo && (
        <div className="repo">
          <p className="pequeño tenue">
            {repo.repo} · versión <span className="mono">{repo.commit.slice(0, 12)}</span> · licencia{" "}
            {repo.license ?? "sin indicar"}
          </p>
          {repo.gated && (
            <p className="pequeño tono-aviso">
              Este repositorio exige aceptar condiciones con una cuenta de Hugging Face: Locall todavía no puede
              descargarlo.
            </p>
          )}
          {repo.files.length === 0 && <p className="pequeño tenue">No tiene modelos GGUF.</p>}
          <ul className="archivos">
            {repo.files.map((f) => {
              const i = instalando[f.path];
              const enCurso = i && !i.error && !i.hecho;
              const pct = i?.progreso && i.progreso.total_bytes > 0
                ? Math.floor((i.progreso.downloaded_bytes / i.progreso.total_bytes) * 100)
                : 0;
              return (
                <li key={f.path} className="archivo">
                  <div className="archivo-fila">
                    <span className="mono">{f.path}</span>
                    <span className="mono tenue">{tamano(f.size_bytes)}</span>
                    <span className="hueco" />
                    {enCurso ? (
                      <button type="button" className="boton" onClick={() => cancelarInstalacion(repo.repo, f.path)}>
                        Cancelar
                      </button>
                    ) : (
                      <button type="button" className="boton" disabled={repo.gated || !!i?.hecho}
                        onClick={() => instalar(repo.repo, f.path)}>
                        {i?.error ? "Reintentar" : "Instalar"}
                      </button>
                    )}
                  </div>
                  {enCurso && (
                    <div className="progreso">
                      <div className="progreso-barra" role="progressbar" aria-valuemin={0} aria-valuemax={100}
                        aria-valuenow={pct} aria-label={`Descargando ${f.path}`}>
                        <div style={{ width: `${pct}%` }} />
                      </div>
                      <span className="mono pequeño">
                        {i?.progreso
                          ? `${pct} % · ${tamano(i.progreso.downloaded_bytes)} de ${tamano(i.progreso.total_bytes)}` +
                            (i.progreso.parts > 1 ? ` · parte ${i.progreso.part}/${i.progreso.parts}` : "")
                          : "Comprobando…"}
                      </span>
                    </div>
                  )}
                  {i?.hecho && <p className="pequeño" role="status">{i.hecho}</p>}
                  {i?.error && <p className="pequeño tono-error" role="alert">{i.error}</p>}
                </li>
              );
            })}
          </ul>
        </div>
      )}
    </section>
  );
}
