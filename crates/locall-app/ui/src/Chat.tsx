// Chat: conversar con un modelo instalado. Las conversaciones las guarda Locall (parte 5d): a la izquierda, la lista;
// al abrir una, se cargan sus mensajes y se puede seguir donde se dejó.

import { type FormEvent, type KeyboardEvent, useEffect, useRef, useState } from "react";

import { BotonCopiar } from "./comun";
import { Formato } from "./Formato";
import {
  type Conversacion,
  type EstadoLocall,
  type FinRespuesta,
  type MensajeGuardado,
  type Modelo,
  abrirConversacion,
  conversar,
  crearConversacion,
  decimal,
  eliminarConversacion,
  grupoDeFecha,
  parar,
  renombrarConversacion,
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
const TEMPERATURA_INICIAL = 0.7;

/** Longitud del título automático (el principio del primer mensaje). */
const TITULO_AUTOMATICO = 60;

interface Props {
  estado: EstadoLocall | null;
  modelos: Modelo[] | null;
  irABiblioteca: () => void;
  /** Las conversaciones guardadas (`null` mientras se cargan). */
  conversaciones: Conversacion[] | null;
  recargarConversaciones: () => void;
  /** La conversación abierta (`null`: una nueva, aún sin guardar). */
  abierta: number | null;
  abrir: (id: number | null) => void;
  /** Avisa de si hay una respuesta en marcha (mientras tanto no se cambia de conversación). */
  alCambiarOcupado: (ocupado: boolean) => void;
  /** Cada vez que cambia, empezar una conversación nueva (Ctrl+N). */
  pedidoNueva: number;
}

export function Chat(props: Props) {
  const { estado, modelos, irABiblioteca, conversaciones, recargarConversaciones, abierta, abrir } = props;
  const usables = (modelos ?? []).filter((m) => m.compatible !== false);
  const [modelo, setModelo] = useState<string>("");
  const [burbujas, setBurbujas] = useState<Burbuja[]>([]);
  const [titulo, setTitulo] = useState<string | null>(null);
  const [entrada, setEntrada] = useState("");
  const [generando, setGenerando] = useState<string | null>(null);
  const [leyendo, setLeyendo] = useState<string | null>(null);
  const [temperatura, setTemperatura] = useState(TEMPERATURA_INICIAL);
  const [sistema, setSistema] = useState("");
  const [errorCarga, setErrorCarga] = useState<string | null>(null);
  /** En ventanas estrechas, el panel de ajustes se abre por encima de la conversación. */
  const [ajustesAbiertos, setAjustesAbiertos] = useState(false);
  const lista = useRef<HTMLDivElement>(null);
  /** La conversación cuyos mensajes están en pantalla (para no recargar la que se acaba de crear al enviar). */
  const enPantalla = useRef<number | null>(null);

  useEffect(() => props.alCambiarOcupado(!!generando), [generando]); // (solo al empezar o terminar)

  /** Empieza una conversación nueva y deja el cursor en el cuadro del mensaje. */
  const nueva = () => {
    if (generando) return;
    // (si la conversación en pantalla no llegó a guardarse, por un error, basta con vaciarla)
    if (abierta === null) {
      setBurbujas([]);
      setTitulo(null);
      setErrorCarga(null);
    } else {
      abrir(null);
    }
    requestAnimationFrame(() => document.getElementById("mensaje")?.focus());
  };

  // Ctrl+N (lo detecta App y avisa subiendo `pedidoNueva`).
  useEffect(() => {
    if (props.pedidoNueva > 0) nueva();
  }, [props.pedidoNueva]); // (solo cuando se pide)

  // Modelo por defecto: el cargado, si está instalado; si no, el primero.
  useEffect(() => {
    if (modelo && usables.some((m) => m.id === modelo)) return;
    const cargado = usables.find((m) => m.id === estado?.model);
    setModelo(cargado?.id ?? usables[0]?.id ?? "");
  }, [modelos, estado?.model]); // (solo al cambiar la lista o el modelo cargado)

  // Al abrir otra conversación: cargar sus mensajes y sus ajustes.
  useEffect(() => {
    if (abierta === enPantalla.current) return;
    enPantalla.current = abierta;
    setErrorCarga(null);
    setLeyendo(null);
    if (abierta === null) {
      setBurbujas([]);
      setTitulo(null);
      return;
    }
    let vigente = true;
    abrirConversacion(abierta)
      .then((c) => {
        if (!vigente) return;
        setBurbujas(c.messages.map(burbujaGuardada));
        setTitulo(c.title);
        setSistema(c.system);
        setTemperatura(c.temperature ?? TEMPERATURA_INICIAL);
        if (c.model && usables.some((m) => m.id === c.model)) setModelo(c.model);
        requestAnimationFrame(() => lista.current && (lista.current.scrollTop = lista.current.scrollHeight));
      })
      .catch((e) => {
        if (!vigente) return;
        setBurbujas([]);
        setTitulo(null);
        setErrorCarga(textoError(e));
      });
    return () => {
      vigente = false;
    };
  }, [abierta]);

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
    let llegoAlgo = false;
    try {
      // Una conversación nueva se guarda al enviar el primer mensaje (así no quedan conversaciones vacías).
      let id = abierta;
      if (id === null) {
        const nueva = await crearConversacion(pregunta.slice(0, TITULO_AUTOMATICO));
        id = nueva.id;
        enPantalla.current = id;
        setTitulo(nueva.title);
        abrir(id);
      }
      const fin: FinRespuesta = await conversar(
        { peticion, conversacion: id, contenido: pregunta, modelo, sistema: sistema.trim(), temperatura },
        (trozo) => {
          if (trozo.type === "prompt_progress") {
            setLeyendo(
              trozo.total > 0 ? `Leyendo la conversación… ${Math.floor((trozo.processed / trozo.total) * 100)} %` : null,
            );
            return;
          }
          setLeyendo(null);
          llegoAlgo = true;
          primerToken ??= performance.now();
          cambiarUltima((b) =>
            trozo.type === "text"
              ? { ...b, texto: b.texto + trozo.text }
              : { ...b, razonamiento: b.razonamiento + trozo.text },
          );
        },
      );
      if (fin.finish === "cancelled" && !llegoAlgo) {
        // Se paró antes de que llegara nada: Locall no guarda la pregunta, así que se quita y vuelve al cuadro.
        setBurbujas((actual) => actual.slice(0, -2));
        setEntrada((actual) => actual || pregunta);
      } else {
        cambiarUltima((b) => ({ ...b, cortada: fin.finish === "cancelled", cifras: cifras(fin, inicio, primerToken) }));
      }
    } catch (error) {
      // Locall no guarda una pregunta que falló: se ve el error aquí y, al reintentar, no queda repetida.
      cambiarUltima((b) => ({ ...b, error: textoError(error) }));
    } finally {
      setGenerando(null);
      setLeyendo(null);
      recargarConversaciones();
    }
  };

  // Enter envía; Mayúsculas+Enter, salto de línea.
  const alTeclear = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      enviar();
    }
  };

  if (modelos !== null && usables.length === 0 && (conversaciones?.length ?? 0) === 0) {
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
      <ListaDeConversaciones
        conversaciones={conversaciones}
        abierta={abierta}
        ocupado={!!generando}
        abrir={abrir}
        nueva={nueva}
        alCambiar={(c) => c.id === abierta && setTitulo(c.title)}
        alEliminar={(id) => {
          if (id === abierta) abrir(null);
          recargarConversaciones();
        }}
        recargar={recargarConversaciones}
      />

      <section className="conversacion" aria-label="Conversación">
        <header className="cabecera-chat">
          <h1 className="titulo-chat">{titulo ?? "Nueva conversación"}</h1>
          <button type="button" className="boton boton-ajustes-chat" aria-expanded={ajustesAbiertos}
            aria-controls="ajustes-chat" onClick={() => setAjustesAbiertos((a) => !a)}>
            Ajustes
          </button>
          <label className="solo-lectores" htmlFor="modelo">Modelo</label>
          <select id="modelo" className="selector" value={modelo} disabled={!!generando || usables.length === 0}
            onChange={(e) => setModelo(e.target.value)}>
            {usables.map((m) => (
              <option key={m.id} value={m.id}>{m.id}{m.quantization ? ` · ${m.quantization}` : ""}</option>
            ))}
          </select>
        </header>

        <div className="mensajes" ref={lista} aria-live="polite" aria-busy={!!generando}>
          {errorCarga && <p className="tono-error" role="alert">No se pudo abrir la conversación: {errorCarga}</p>}
          {!errorCarga && burbujas.length === 0 && <p className="tenue vacio">Escribe abajo para empezar.</p>}
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
                {(b.cifras || (b.rol === "assistant" && b.texto && !escribiendo)) && (
                  <div className="pie-mensaje">
                    {b.cifras && <span className="mono cifras">{b.cifras}</span>}
                    {b.rol === "assistant" && b.texto && !escribiendo && (
                      <BotonCopiar texto={b.texto} etiqueta="Copiar la respuesta" />
                    )}
                  </div>
                )}
              </article>
            );
          })}
        </div>

        <form className="redactar" onSubmit={enviar}>
          <label htmlFor="mensaje" className="pequeño tenue">Mensaje</label>
          <textarea id="mensaje" className="campo-texto" rows={2} value={entrada}
            placeholder={usables.length === 0 ? "Instala un modelo para seguir conversando"
              : "Escribe tu mensaje (Enter envía, Mayús+Enter salta de línea)"}
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

      <aside id="ajustes-chat" className={`ajustes-chat${ajustesAbiertos ? " abierto" : ""}`}
        aria-label="Ajustes de la conversación">
        <div className="fila-etiqueta">
          <h2>Ajustes</h2>
          <button type="button" className="enlace cerrar-ajustes" onClick={() => setAjustesAbiertos(false)}>
            Cerrar
          </button>
        </div>
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
          <span className="pequeño tenue">Se guardan con la conversación al enviar el siguiente mensaje.</span>
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

// ─── La lista de la izquierda ─────────────────────────────────────────────────

interface PropsLista {
  conversaciones: Conversacion[] | null;
  abierta: number | null;
  ocupado: boolean;
  abrir: (id: number | null) => void;
  nueva: () => void;
  alCambiar: (c: Conversacion) => void;
  alEliminar: (id: number) => void;
  recargar: () => void;
}

function ListaDeConversaciones(props: PropsLista) {
  const { conversaciones, abierta, ocupado, abrir, nueva, alCambiar, alEliminar, recargar } = props;
  const [renombrando, setRenombrando] = useState<number | null>(null);
  const [nuevoTitulo, setNuevoTitulo] = useState("");
  const [confirmando, setConfirmando] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Agrupadas por fecha (la lista ya llega de la más reciente a la más antigua).
  const grupos: { nombre: string; lista: Conversacion[] }[] = [];
  for (const c of conversaciones ?? []) {
    const nombre = grupoDeFecha(c.updated_at);
    if (grupos.at(-1)?.nombre !== nombre) grupos.push({ nombre, lista: [] });
    grupos[grupos.length - 1].lista.push(c);
  }

  const guardarTitulo = async (c: Conversacion) => {
    const t = nuevoTitulo.trim();
    setRenombrando(null);
    if (!t || t === c.title) return;
    try {
      alCambiar(await renombrarConversacion(c.id, t));
      setError(null);
    } catch (e) {
      setError(textoError(e));
    }
    recargar();
  };

  const eliminar = async (id: number) => {
    setConfirmando(null);
    try {
      await eliminarConversacion(id);
      setError(null);
      alEliminar(id);
    } catch (e) {
      setError(textoError(e));
    }
  };

  return (
    <nav className="conversaciones" aria-label="Conversaciones guardadas">
      <button type="button" className="boton ancho" disabled={ocupado} onClick={nueva} aria-keyshortcuts="Control+N"
        title="Nueva conversación (Ctrl+N)">
        + Nueva conversación
      </button>
      {error && <p className="pequeño tono-error" role="alert">{error}</p>}
      {conversaciones === null && <p className="pequeño tenue">Cargando…</p>}
      <div className="lista-conversaciones">
        {grupos.map((g) => (
          <section key={g.nombre} aria-label={g.nombre}>
            <h2 className="grupo-fecha">{g.nombre}</h2>
            <ul>
              {g.lista.map((c) => {
                const activa = c.id === abierta;
                return (
                  <li key={c.id} className={`elemento-conversacion${activa ? " activa" : ""}`}>
                    {renombrando === c.id ? (
                      <input className="campo renombrar" aria-label="Nuevo título" autoFocus value={nuevoTitulo}
                        maxLength={120} onChange={(e) => setNuevoTitulo(e.target.value)}
                        onBlur={() => guardarTitulo(c)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") guardarTitulo(c);
                          if (e.key === "Escape") setRenombrando(null);
                        }} />
                    ) : (
                      <button type="button" className="abrir-conversacion" title={c.title}
                        aria-current={activa ? "true" : undefined} disabled={ocupado && !activa}
                        onClick={() => abrir(c.id)}>
                        {c.title}
                      </button>
                    )}
                    {activa && renombrando !== c.id && (
                      confirmando === c.id ? (
                        <span className="acciones-conversacion">
                          <span className="pequeño">¿Eliminar para siempre?</span>
                          <button type="button" className="enlace peligro" onClick={() => eliminar(c.id)}>Eliminar</button>
                          <button type="button" className="enlace" onClick={() => setConfirmando(null)}>Cancelar</button>
                        </span>
                      ) : (
                        <span className="acciones-conversacion">
                          <button type="button" className="enlace" disabled={ocupado}
                            onClick={() => (setNuevoTitulo(c.title), setRenombrando(c.id))}>
                            Renombrar
                          </button>
                          <button type="button" className="enlace" disabled={ocupado}
                            onClick={() => setConfirmando(c.id)}>
                            Eliminar
                          </button>
                        </span>
                      )
                    )}
                  </li>
                );
              })}
            </ul>
          </section>
        ))}
      </div>
    </nav>
  );
}

// ─── Ayudantes ────────────────────────────────────────────────────────────────

/** Un mensaje guardado, como burbuja. */
function burbujaGuardada(m: MensajeGuardado): Burbuja {
  const partes: string[] = [];
  if (m.generation_per_second) partes.push(`${decimal(m.generation_per_second)} t/s`);
  if (m.completion_tokens) partes.push(`${m.completion_tokens} tokens`);
  return {
    rol: m.role,
    texto: m.content,
    razonamiento: m.reasoning,
    modelo: m.model ?? undefined,
    cortada: m.truncated,
    cifras: m.role === "assistant" && !m.truncated && partes.length > 0 ? partes.join(" · ") : undefined,
  };
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
