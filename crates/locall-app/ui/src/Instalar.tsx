// Instalar modelos de Hugging Face: el estado de las instalaciones en marcha y la lista de archivos de un repositorio.
//
// El estado vive en `App` (con `useInstalaciones`) y no en cada pantalla: así, si empiezas a instalar en Explorar y te
// vas a la Biblioteca, el progreso sigue a la vista y la instalación no se pierde.

import { useCallback, useState } from "react";

import {
  type Progreso,
  type Repositorio,
  cancelarInstalacion,
  instalarDelCatalogo,
  instalarModelo,
  tamano,
  textoError,
} from "./locall";

/** Cómo va una instalación. */
export interface Instalando {
  progreso: Progreso | null;
  error: string | null;
  hecho: string | null;
}

/** Las instalaciones (clave: `repo/archivo`) y cómo empezarlas o cancelarlas. */
export interface Instalaciones {
  estado: Record<string, Instalando>;
  instalar: (repo: string, archivo: string) => void;
  cancelar: (repo: string, archivo: string) => void;
  /** Instala un modelo del catálogo (su progreso va en `estado[clave("catalogo", nombre)]`). */
  instalarDelCatalogo: (nombre: string) => void;
}

export const clave = (repo: string, archivo: string) => `${repo}/${archivo}`;

export function useInstalaciones(alInstalar: () => void): Instalaciones {
  const [estado, setEstado] = useState<Record<string, Instalando>>({});

  /** Lleva el estado de una instalación (`k`) mientras `trabajo` la hace. */
  const seguir = useCallback(
    async (k: string, trabajo: (alProgresar: (p: Progreso) => void) => Promise<{ id: string; size_bytes: number }>) => {
      const poner = (cambio: Partial<Instalando>) =>
        setEstado((actual) => ({
          ...actual,
          [k]: { ...(actual[k] ?? { progreso: null, error: null, hecho: null }), ...cambio },
        }));
      poner({ progreso: null, error: null, hecho: null });
      try {
        const modelo = await trabajo((p) => poner({ progreso: p }));
        poner({ hecho: `Instalado «${modelo.id}» (${tamano(modelo.size_bytes)}, verificado con sha256).` });
        alInstalar();
      } catch (e) {
        poner({ error: textoError(e) });
      }
    },
    [alInstalar],
  );

  const instalar = useCallback(
    (repo: string, archivo: string) => seguir(clave(repo, archivo), (p) => instalarModelo(repo, archivo, p)),
    [seguir],
  );

  const instalarCatalogo = useCallback(
    (nombre: string) => seguir(clave("catalogo", nombre), (p) => instalarDelCatalogo(nombre, p)),
    [seguir],
  );

  const cancelar = useCallback((repo: string, archivo: string) => {
    cancelarInstalacion(repo, archivo);
  }, []);

  return { estado, instalar, cancelar, instalarDelCatalogo: instalarCatalogo };
}

/** Los GGUF de un repositorio, cada uno con su botón de instalar y su progreso. */
export function ArchivosDelRepo({ repo, instalaciones }: { repo: Repositorio; instalaciones: Instalaciones }) {
  return (
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
          const i = instalaciones.estado[clave(repo.repo, f.path)];
          const enCurso = i && !i.error && !i.hecho;
          const pct =
            i?.progreso && i.progreso.total_bytes > 0
              ? Math.floor((i.progreso.downloaded_bytes / i.progreso.total_bytes) * 100)
              : 0;
          return (
            <li key={f.path} className="archivo">
              <div className="archivo-fila">
                <span className="mono">{f.path}</span>
                <span className="mono tenue">{tamano(f.size_bytes)}</span>
                <span className="hueco" />
                {enCurso ? (
                  <button type="button" className="boton" onClick={() => instalaciones.cancelar(repo.repo, f.path)}>
                    Cancelar
                  </button>
                ) : (
                  <button type="button" className="boton" disabled={repo.gated || !!i?.hecho}
                    onClick={() => instalaciones.instalar(repo.repo, f.path)}>
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
  );
}
