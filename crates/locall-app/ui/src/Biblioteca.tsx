// Biblioteca: los modelos instalados (con eliminar) y añadir uno desde Hugging Face.

import { type FormEvent, useState } from "react";

import { EstadoDelModelo, NombreDelModelo } from "./comun";
import { ArchivosDelRepo, type Instalaciones } from "./Instalar";
import {
  type EstadoLocall,
  type Modelo,
  type Repositorio,
  consultarRepo,
  eliminarModelo,
  importarGguf,
  tamano,
  textoError,
} from "./locall";

interface Props {
  estado: EstadoLocall | null;
  modelos: Modelo[] | null;
  recargar: () => void;
}

export function Biblioteca(props: Props & { instalaciones: Instalaciones; irAExplorar: () => void }) {
  return (
    <>
      <h1>Biblioteca</h1>
      <Instalados estado={props.estado} modelos={props.modelos} recargar={props.recargar} />
      <AnadirDesdeHuggingFace instalaciones={props.instalaciones} irAExplorar={props.irAExplorar} />
      <ImportarDelEquipo recargar={props.recargar} />
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

function AnadirDesdeHuggingFace({ instalaciones, irAExplorar }: { instalaciones: Instalaciones; irAExplorar: () => void }) {
  const [texto, setTexto] = useState("");
  const [buscando, setBuscando] = useState(false);
  const [repo, setRepo] = useState<Repositorio | null>(null);
  const [error, setError] = useState<string | null>(null);

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

  return (
    <section className="tarjeta ancha" aria-labelledby="titulo-anadir">
      <h2 id="titulo-anadir">Añadir desde Hugging Face</h2>
      <p className="pequeño tenue">
        Escribe el repositorio (propietario/nombre). Locall fija la versión exacta, comprueba que el motor puede usar el
        modelo antes de descargarlo y verifica cada archivo con sha256. ¿No sabes el nombre exacto?{" "}
        <button type="button" className="enlace" onClick={irAExplorar}>Búscalo en Explorar</button>.
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
      {repo && <ArchivosDelRepo repo={repo} instalaciones={instalaciones} />}
    </section>
  );
}

// ─── Importar un archivo del equipo ───────────────────────────────────────────

function ImportarDelEquipo({ recargar }: { recargar: () => void }) {
  const [importando, setImportando] = useState<string | null>(null);
  const [abierta, setAbierta] = useState(false);
  const [mensaje, setMensaje] = useState<{ texto: string; error: boolean } | null>(null);

  const importar = async () => {
    setMensaje(null);
    setAbierta(true);
    try {
      const modelo = await importarGguf((nombre) => setImportando(nombre));
      if (modelo) {
        setMensaje({ texto: `Importado «${modelo.id}» (${tamano(modelo.size_bytes)}).`, error: false });
        recargar();
      }
    } catch (e) {
      setMensaje({ texto: textoError(e), error: true });
    } finally {
      setAbierta(false);
      setImportando(null);
    }
  };

  return (
    <section className="tarjeta ancha" aria-labelledby="titulo-importar">
      <h2 id="titulo-importar">Importar desde tu equipo</h2>
      <p className="pequeño tenue">
        ¿Ya tienes un archivo <span className="mono">.gguf</span>? Locall comprueba que el motor puede usarlo y lo añade
        sin moverlo. Si está en el mismo disco que Locall, no se copia: no ocupa el doble.
      </p>
      <div className="fila-botones">
        <button type="button" className="boton" disabled={abierta} onClick={importar}>
          {importando ? `Importando ${importando}…` : abierta ? "Elige el archivo…" : "Elegir un archivo .gguf"}
        </button>
      </div>
      {mensaje && (
        <p className={`pequeño${mensaje.error ? " tono-error" : ""}`} role={mensaje.error ? "alert" : "status"}>
          {mensaje.texto}
        </p>
      )}
    </section>
  );
}
