// Ajustes: ver y cambiar `config.toml` sin abrirlo a mano. Los cambios se guardan en el archivo y se aplican al
// reiniciar Locall (ADR 0020).

import { type FormEvent, useEffect, useState } from "react";

import {
  type Ajustes as DatosAjustes,
  type ValoresAjustes,
  abrirCarpeta,
  guardarAjustes,
  pedirAjustes,
  reiniciarLocall,
  textoError,
} from "./locall";

/** Lo que se puede cambiar aquí (como texto mientras se escribe). */
interface Formulario {
  context: string;
  threadsAuto: boolean;
  threads: string;
  unload_minutes: string;
  exit_minutes: string;
  log_days: string;
}

const deAjustes = (a: ValoresAjustes): Formulario => ({
  context: String(a.context),
  threadsAuto: a.threads === "auto",
  threads: a.threads === "auto" ? "" : String(a.threads),
  unload_minutes: String(a.unload_minutes),
  exit_minutes: String(a.exit_minutes),
  log_days: String(a.log_days),
});

interface Props {
  /** Hay algo en marcha (una respuesta, una instalación, un modelo cargándose): no se puede reiniciar ahora. */
  ocupado: boolean;
}

export function Ajustes({ ocupado }: Props) {
  const [datos, setDatos] = useState<DatosAjustes | null>(null);
  const [form, setForm] = useState<Formulario | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [mensaje, setMensaje] = useState<string | null>(null);
  const [guardando, setGuardando] = useState(false);
  const [reiniciando, setReiniciando] = useState(false);

  const cargar = () =>
    pedirAjustes()
      .then((d) => {
        setDatos(d);
        setForm(deAjustes(d.saved));
      })
      .catch((e) => setError(textoError(e)));
  useEffect(() => {
    cargar();
  }, []);

  if (!datos || !form) {
    return (
      <>
        <h1>Ajustes</h1>
        {error ? <p className="tono-error" role="alert">{error}</p> : <p className="tenue">Cargando…</p>}
      </>
    );
  }

  const poner = (cambio: Partial<Formulario>) => {
    setForm({ ...form, ...cambio });
    setMensaje(null);
  };

  // Solo se envía lo que ha cambiado respecto a lo guardado.
  const cambios: Record<string, number | "auto"> = {};
  const numero = (campo: "context" | "unload_minutes" | "exit_minutes" | "log_days") => {
    const n = Number(form[campo]);
    if (form[campo].trim() !== "" && n !== datos.saved[campo]) cambios[campo] = n;
  };
  numero("context");
  numero("unload_minutes");
  numero("exit_minutes");
  numero("log_days");
  const hilos = form.threadsAuto ? "auto" : Number(form.threads);
  if (hilos !== datos.saved.threads && (form.threadsAuto || form.threads.trim() !== "")) cambios.threads = hilos;
  const hayCambios = Object.keys(cambios).length > 0;

  const guardar = async (e: FormEvent) => {
    e.preventDefault();
    if (!hayCambios) return;
    setGuardando(true);
    setError(null);
    try {
      const d = await guardarAjustes(cambios);
      setDatos(d);
      setForm(deAjustes(d.saved));
      setMensaje("Guardado en config.toml.");
    } catch (e) {
      setError(textoError(e));
    } finally {
      setGuardando(false);
    }
  };

  const reiniciar = async () => {
    setReiniciando(true);
    setError(null);
    try {
      await reiniciarLocall();
      // Esperar a que vuelva a abrirse (lo abre la siguiente petición) y releer.
      await new Promise((r) => setTimeout(r, 1500));
      await cargar();
      setMensaje("Locall se ha reiniciado con los ajustes nuevos.");
    } catch (e) {
      setError(textoError(e));
    } finally {
      setReiniciando(false);
    }
  };

  const abrir = (cual: "datos" | "modelos") => abrirCarpeta(cual).catch((e) => setError(textoError(e)));

  return (
    <>
      <h1>Ajustes</h1>

      {datos.restart_needed && (
        <section className="tarjeta aviso ancha" role="status">
          <p>
            Hay ajustes guardados que todavía no se usan: se aplican al reiniciar Locall (tus modelos y conversaciones
            no se tocan).
          </p>
          <button type="button" className="boton primario" disabled={ocupado || reiniciando} onClick={reiniciar}>
            {reiniciando ? "Reiniciando…" : "Reiniciar Locall ahora"}
          </button>
          {ocupado && (
            <span className="pequeño tenue">
              {" "}Espera a que termine lo que está en marcha (una respuesta, una instalación…).
            </span>
          )}
        </section>
      )}

      <form className="tarjeta ancha ajustes" onSubmit={guardar} aria-labelledby="titulo-motor">
        <h2 id="titulo-motor">Modelos y memoria</h2>

        <div className="ajuste">
          <label htmlFor="contexto">Contexto (tokens)</label>
          <input id="contexto" className="campo mono" type="number" min={256} max={1048576} step={256}
            value={form.context} onChange={(e) => poner({ context: e.target.value })} />
          <span className="pequeño tenue">
            Cuánto de la conversación «recuerda» el modelo. Más contexto, más memoria. 4096 va bien para empezar.
          </span>
        </div>

        <fieldset className="ajuste">
          <legend>Hilos del procesador</legend>
          <div className="fila-opciones">
            <label className="opcion">
              <input type="radio" name="hilos" checked={form.threadsAuto} onChange={() => poner({ threadsAuto: true })} />
              Todos ({datos.threads_available})
            </label>
            <label className="opcion">
              <input type="radio" name="hilos" checked={!form.threadsAuto}
                onChange={() => poner({ threadsAuto: false, threads: form.threads || String(datos.threads_available) })} />
              Solo
            </label>
            <input className="campo mono corto" type="number" min={1} max={datos.threads_available} aria-label="Número de hilos"
              disabled={form.threadsAuto} value={form.threads} onChange={(e) => poner({ threads: e.target.value })} />
          </div>
          <span className="pequeño tenue">Menos hilos deja el equipo más libre mientras el modelo responde, pero va más lento.</span>
        </fieldset>

        <div className="ajuste">
          <label htmlFor="liberar">Liberar la memoria del modelo tras (minutos sin usarlo)</label>
          <input id="liberar" className="campo mono corto" type="number" min={0} max={10080}
            value={form.unload_minutes} onChange={(e) => poner({ unload_minutes: e.target.value })} />
          <span className="pequeño tenue">0 = no liberarla nunca.</span>
        </div>

        <div className="ajuste">
          <label htmlFor="cerrar">Cerrar Locall en segundo plano tras (minutos sin usarlo)</label>
          <input id="cerrar" className="campo mono corto" type="number" min={0} max={10080}
            value={form.exit_minutes} onChange={(e) => poner({ exit_minutes: e.target.value })} />
          <span className="pequeño tenue">0 = no cerrarlo nunca. La app y la línea de comandos lo vuelven a abrir solas.</span>
        </div>

        <div className="ajuste">
          <label htmlFor="registros">Guardar los registros (días)</label>
          <input id="registros" className="campo mono corto" type="number" min={1} max={365}
            value={form.log_days} onChange={(e) => poner({ log_days: e.target.value })} />
          <span className="pequeño tenue">Los registros nunca contienen tus conversaciones.</span>
        </div>

        {error && <p className="pequeño tono-error" role="alert">{error}</p>}
        {mensaje && <p className="pequeño" role="status">{mensaje}</p>}
        <div className="fila-botones">
          <button type="submit" className="boton primario" disabled={!hayCambios || guardando}>
            {guardando ? "Guardando…" : "Guardar"}
          </button>
          <button type="button" className="boton" disabled={!hayCambios || guardando}
            onClick={() => (setForm(deAjustes(datos.saved)), setMensaje(null), setError(null))}>
            Deshacer
          </button>
        </div>
      </form>

      <section className="tarjeta ancha ajustes" aria-labelledby="titulo-carpetas">
        <h2 id="titulo-carpetas">Carpetas y conexión</h2>
        <div className="dato-ajuste">
          <span className="tenue pequeño">Carpeta de los modelos</span>
          <span className="mono ruta">{datos.active.models_folder}</span>
          <button type="button" className="enlace" onClick={() => abrir("modelos")}>Abrir</button>
        </div>
        <div className="dato-ajuste">
          <span className="tenue pequeño">Carpeta de datos (ajustes, conversaciones, registros)</span>
          <span className="mono ruta">{datos.data_folder}</span>
          <button type="button" className="enlace" onClick={() => abrir("datos")}>Abrir</button>
        </div>
        <div className="dato-ajuste">
          <span className="tenue pequeño">Puerto de la API local</span>
          <span className="mono">127.0.0.1:{datos.active.port}</span>
        </div>
        <p className="pequeño tenue">
          La carpeta de los modelos y el puerto se cambian a mano en <span className="mono">config.toml</span> (en la
          carpeta de datos), con Locall cerrado: cambiarlos con Locall abierto dejaría tus modelos o la conexión en el
          sitio antiguo.
        </p>
      </section>

      <section className="tarjeta ancha" aria-labelledby="titulo-privacidad">
        <h2 id="titulo-privacidad">Privacidad</h2>
        <p className="pequeño">
          Locall funciona en tu equipo. Solo se conecta a internet para buscar y descargar modelos de Hugging Face. No
          envía estadísticas ni tus conversaciones a ningún sitio, y no hay nada que desactivar.
        </p>
      </section>
    </>
  );
}
