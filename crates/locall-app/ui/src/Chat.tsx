// Chat: conversar con un modelo instalado. La conversación se recuerda mientras está abierta (guardarla es la 5d).

import { type FormEvent, type KeyboardEvent, useEffect, useRef, useState } from "react";

import { Formato } from "./Formato";
import {
  type EstadoLocall,
  type FinRespuesta,
  type Mensaje,
  type Modelo,
  conversar,
  decimal,
  parar,
  textoError,
} from "./locall";

/** Un mensaje tal como se muestra. */
interface Burbuja {
  rol: "user" | "assistant";
  texto: string;
  razonamiento: string;
  /** Solo en las respuestas: el modelo que respondió y sus cifras. */
  modelo?: string;
  cifras?: string;
  error?: string;
  cortada?: boolean;
}

/** Estilos de respuesta de la maqueta: atajos para la temperatura. */
const ESTILOS = [
  { nombre: "Preciso", temperatura: 0.2 },
  { nombre: "Equilibrado", temperatura: 0.7 },
  { nombre: "Creativo", temperatura: 1.1 },
] as const;

interface Props {
  estado: EstadoLocall | null;
  modelos: Modelo[] | null;
  irABiblioteca: () => void;
}

export function Chat({ estado, modelos, irABiblioteca }: Props) {
  const usables = (modelos ?? []).filter((m) => m.compatible !== false);
  const [modelo, setModelo] = useState<string>("");
  const [burbujas, setBurbujas] = useState<Burbuja[]>([]);
  const [entrada, setEntrada] = useState("");
  const [generando, setGenerando] = useState<string | null>(null);
  const [leyendo, setLeyendo] = useState<string | null>(null);
  const [temperatura, setTemperatura] = useState(0.7);
  const [sistema, setSistema] = useState("");
  const lista = useRef<HTMLDivElement>(null);

  // Modelo por defecto: el cargado, si está instalado; si no, el primero.
  useEffect(() => {
    if (modelo && usables.some((m) => m.id === modelo)) return;
    const cargado = usables.find((m) => m.id === estado?.model);
    setModelo(cargado?.id ?? usables[0]?.id ?? "");
  }, [modelos, estado?.model]); // (solo al cambiar la lista o el modelo cargado)

  // Bajar hasta el final cuando llega texto (si el usuario ya estaba abajo).
  useEffect(() => {
    const el = lista.current;
    if (el && el.scrollHeight - el.scrollTop - el.clientHeight < 120) el.scrollTop = el.scrollHeight;
  }, [burbujas]);

  // Esc para parar la respuesta.
  useEffect(() => {
    if (!generando) return;
    const tecla = (e: globalThis.KeyboardEvent) => {
      if (e.key === "Escape") parar(generando);
    };
    window.addEventListener("keydown", tecla);
    return () => window.removeEventListener("keydown", tecla);
  }, [generando]);

  /** Cambia la última burbuja (la respuesta que se está escribiendo). */
  const cambiarUltima = (cambio: (b: Burbuja) => Burbuja) =>
    setBurbujas((actual) => [...actual.slice(0, -1), cambio(actual[actual.length - 1])]);

  const enviar = async (e?: FormEvent) => {
    e?.preventDefault();
    const pregunta = entrada.trim();
    if (!pregunta || !modelo || generando) return;

    // La conversación que se envía: instrucciones del sistema + lo anterior (sin errores) + la pregunta nueva.
    const historial: Mensaje[] = [];
    if (sistema.trim()) historial.push({ role: "system", content: sistema.trim() });
    for (const b of burbujas) {
      if (!b.error && b.texto) historial.push({ role: b.rol, content: b.texto });
    }
    historial.push({ role: "user", content: pregunta });

    const peticion = crypto.randomUUID();
    setEntrada("");
    setGenerando(peticion);
    setBurbujas((actual) => [
      ...actual,
      { rol: "user", texto: pregunta, razonamiento: "" },
      { rol: "assistant", texto: "", razonamiento: "", modelo },
    ]);
    const inicio = performance.now();
    let primerToken: number | null = null;
    try {
      const fin: FinRespuesta = await conversar(peticion, modelo, historial, temperatura, (trozo) => {
        if (trozo.type === "prompt_progress") {
          setLeyendo(trozo.total > 0 ? `Leyendo la conversación… ${Math.floor((trozo.processed / trozo.total) * 100)} %` : null);
          return;
        }
        setLeyendo(null);
        primerToken ??= performance.now();
        cambiarUltima((b) =>
          trozo.type === "text" ? { ...b, texto: b.texto + trozo.text } : { ...b, razonamiento: b.razonamiento + trozo.text },
        );
      });
      cambiarUltima((b) => ({ ...b, cortada: fin.finish === "cancelled", cifras: cifras(fin, inicio, primerToken) }));
    } catch (error) {
      cambiarUltima((b) => ({ ...b, error: textoError(error) }));
    } finally {
      setGenerando(null);
      setLeyendo(null);
    }
  };

  // Enter envía; Mayúsculas+Enter, salto de línea.
  const alTeclear = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      enviar();
    }
  };

  if (modelos !== null && usables.length === 0) {
    return (
      <>
        <h1>Chat</h1>
        <p className="tenue">
          Para conversar necesitas un modelo.{" "}
          <button type="button" className="enlace" onClick={irABiblioteca}>Instala uno en la biblioteca</button>.
        </p>
      </>
    );
  }

  return (
    <div className="chat">
      <section className="conversaciones" aria-label="Conversaciones">
        <button type="button" className="boton ancho" disabled={!!generando} onClick={() => setBurbujas([])}>
          + Nueva conversación
        </button>
        <p className="pequeño tenue">Guardar las conversaciones llegará en la parte 5d.</p>
      </section>

      <section className="conversacion" aria-label="Conversación">
        <header className="cabecera-chat">
          <h1 className="titulo-chat">{burbujas[0]?.texto.slice(0, 60) || "Nueva conversación"}</h1>
          <label className="solo-lectores" htmlFor="modelo">Modelo</label>
          <select id="modelo" className="selector" value={modelo} disabled={!!generando}
            onChange={(e) => setModelo(e.target.value)}>
            {usables.map((m) => (
              <option key={m.id} value={m.id}>{m.id}{m.quantization ? ` · ${m.quantization}` : ""}</option>
            ))}
          </select>
        </header>

        <div className="mensajes" ref={lista} aria-live="polite">
          {burbujas.length === 0 && <p className="tenue vacio">Escribe abajo para empezar.</p>}
          {burbujas.map((b, i) => {
            const escribiendo = generando && i === burbujas.length - 1;
            return (
              <article key={i} className="mensaje">
                <span className={`autor${b.rol === "assistant" ? " modelo" : ""}`}>
                  {b.rol === "user" ? "Tú" : b.modelo}
                  {escribiendo && " · escribiendo…"}
                </span>
                {b.razonamiento && (
                  <details className="razonamiento" open={!!escribiendo && !b.texto}>
                    <summary>Razonamiento</summary>
                    <Formato texto={b.razonamiento} />
                  </details>
                )}
                {b.rol === "user" ? <p className="parrafo-respuesta">{b.texto}</p> : <Formato texto={b.texto} />}
                {escribiendo && leyendo && <span className="pequeño tenue">{leyendo}</span>}
                {escribiendo && !leyendo && !b.texto && !b.razonamiento && (
                  <span className="pequeño tenue">
                    {estado?.model === modelo ? "Pensando…" : "Cargando el modelo (la primera vez tarda unos segundos)…"}
                  </span>
                )}
                {b.cortada && <span className="pequeño tenue">Respuesta parada.</span>}
                {b.error && <p className="pequeño tono-error" role="alert">{b.error}</p>}
                {b.cifras && <span className="mono cifras">{b.cifras}</span>}
              </article>
            );
          })}
        </div>

        <form className="redactar" onSubmit={enviar}>
          <label htmlFor="mensaje" className="pequeño tenue">Mensaje</label>
          <textarea id="mensaje" className="campo-texto" rows={2} value={entrada}
            placeholder="Escribe tu mensaje (Enter envía, Mayús+Enter salta de línea)"
            onChange={(e) => setEntrada(e.target.value)} onKeyDown={alTeclear} />
          <div className="fila-botones">
            <span className="hueco" />
            {generando && (
              <button type="button" className="boton" onClick={() => parar(generando)}>
                ■ Parar <span className="mono tenue">Esc</span>
              </button>
            )}
            <button type="submit" className="boton primario" disabled={!!generando || !entrada.trim() || !modelo}>
              Enviar
            </button>
          </div>
        </form>
      </section>

      <aside className="ajustes-chat" aria-label="Ajustes de la conversación">
        <h2>Ajustes</h2>
        <fieldset className="grupo">
          <legend className="pequeño tenue">Estilo de respuesta</legend>
          <div className="segmentado">
            {ESTILOS.map((s) => (
              <button key={s.nombre} type="button" aria-pressed={temperatura === s.temperatura}
                className={temperatura === s.temperatura ? "activo" : ""} onClick={() => setTemperatura(s.temperatura)}>
                {s.nombre}
              </button>
            ))}
          </div>
        </fieldset>
        <div className="grupo">
          <div className="fila-etiqueta">
            <label htmlFor="temperatura" className="pequeño tenue">Temperatura</label>
            <span className="mono pequeño">{decimal(temperatura)}</span>
          </div>
          <input id="temperatura" type="range" min={0} max={2} step={0.1} value={temperatura}
            onChange={(e) => setTemperatura(Number(e.target.value))} />
          <span className="pequeño tenue">Más baja: más precisa y repetible. Más alta: más variada.</span>
        </div>
        <div className="grupo">
          <label htmlFor="sistema" className="pequeño tenue">Instrucciones del sistema</label>
          <textarea id="sistema" className="campo-texto" rows={4} value={sistema}
            placeholder="Por ejemplo: Responde en español y explica paso a paso."
            onChange={(e) => setSistema(e.target.value)} />
        </div>
        <span className="hueco" />
        <div className="grupo">
          <span className="pequeño tenue">Equivalente en la línea de comandos</span>
          <code className="cli">locall run {modelo || "<modelo>"}</code>
        </div>
      </aside>
    </div>
  );
}

/** "12,4 t/s · 186 tokens · 15,0 s · primer token 0,8 s" */
function cifras(fin: FinRespuesta, inicio: number, primerToken: number | null): string | undefined {
  if (fin.finish === "cancelled" || !fin.usage) return undefined;
  const partes: string[] = [];
  if (fin.speed?.generation_per_second) partes.push(`${decimal(fin.speed.generation_per_second)} t/s`);
  partes.push(`${fin.usage.completion_tokens} tokens`);
  partes.push(`${decimal((performance.now() - inicio) / 1000)} s`);
  if (primerToken !== null) partes.push(`primer token ${decimal((primerToken - inicio) / 1000)} s`);
  if (fin.finish === "length") partes.push("cortada por el límite de longitud");
  return partes.join(" · ");
}
