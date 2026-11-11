import { type ReactNode, useCallback, useEffect, useState } from "react";

import { Ajustes } from "./Ajustes";
import { ATAJOS_PANTALLA, type Pagina, esAtajoDelNavegador } from "./atajos";
import { Biblioteca } from "./Biblioteca";
import { Chat } from "./Chat";
import { Icono } from "./comun";
import { Explorar } from "./Explorar";
import { Inicio } from "./Inicio";
import { useInstalaciones } from "./Instalar";
import {
  type Conversacion,
  type EstadoLocall,
  type Modelo,
  minutosSegundos,
  pedirConversaciones,
  pedirEstado,
  pedirModelos,
  tamano,
  textoError,
} from "./locall";
import logoClaro from "./assets/logo-claro.png";
import logoOscuro from "./assets/logo-oscuro.png";

/** How often Locall is asked how it is doing (for the status bar). */
const INTERVALO_ESTADO_MS = 2000;

export function App() {
  const [pagina, setPagina] = useState<Pagina>("inicio");
  const [estado, setEstado] = useState<EstadoLocall | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [modelos, setModelos] = useState<Modelo[] | null>(null);
  const [conversaciones, setConversaciones] = useState<Conversacion[] | null>(null);
  /** The conversation open in the chat (`null`: a new one). */
  const [abierta, setAbierta] = useState<number | null>(null);
  /** An answer is in progress: meanwhile the conversation is not changed. */
  const [ocupado, setOcupado] = useState(false);
  /** Every time it goes up, the chat starts a new conversation (Ctrl+N). */
  const [pedidoNueva, setPedidoNueva] = useState(0);

  // Ask for the state at the start and every 2 s. (The first time may take a while: if Locall is not open, it opens.)
  useEffect(() => {
    let vivo = true;
    const preguntar = () =>
      pedirEstado()
        .then((e) => vivo && (setEstado(e), setError(null)))
        .catch((e) => vivo && setError(textoError(e)));
    preguntar();
    const temporizador = setInterval(preguntar, INTERVALO_ESTADO_MS);
    return () => {
      vivo = false;
      clearInterval(temporizador);
    };
  }, []);

  // The model list: at the start, when changing screens and when one is installed or removed.
  const recargarModelos = useCallback(() => {
    pedirModelos()
      .then(setModelos)
      .catch(() => setModelos((actual) => actual ?? []));
  }, []);
  useEffect(recargarModelos, [pagina, recargarModelos]);
  const instalaciones = useInstalaciones(recargarModelos);

  // The saved conversations: at the start, when changing screens and when the chat changes something.
  const recargarConversaciones = useCallback(() => {
    pedirConversaciones()
      .then(setConversaciones)
      .catch(() => setConversaciones((actual) => actual ?? []));
  }, []);
  useEffect(recargarConversaciones, [pagina, recargarConversaciones]);

  // Keyboard shortcuts (see atajos.ts).
  useEffect(() => {
    const alTeclear = (e: KeyboardEvent) => {
      if (import.meta.env.PROD && esAtajoDelNavegador(e)) {
        e.preventDefault();
        return;
      }
      if (!e.ctrlKey || e.altKey || e.metaKey || e.isComposing) return;
      const tecla = e.key.toLowerCase();
      // (also by the key's position, in case the keyboard layout gives another character)
      const pantalla = ATAJOS_PANTALLA.find(
        (a) => a.tecla === tecla || (a.tecla === "," && e.code === "Comma") || e.code === `Digit${a.tecla}`,
      );
      if (pantalla) {
        e.preventDefault();
        setPagina(pantalla.pagina);
      } else if (tecla === "n" && !e.shiftKey) {
        e.preventDefault(); // (otherwise WebView2 would open a new window)
        setPagina("chat");
        setPedidoNueva((n) => n + 1);
      } else if (tecla === "k") {
        e.preventDefault();
        setPagina("explorar");
        enfocar("buscar");
      }
    };
    window.addEventListener("keydown", alTeclear);
    return () => window.removeEventListener("keydown", alTeclear);
  }, []);

  /** Opens a conversation in the chat (from Home, «Continuar»). */
  const continuar = (id: number) => {
    if (!ocupado) setAbierta(id);
    setPagina("chat");
  };

  return (
    <div className="app">
      <div className="cuerpo">
        <BarraLateral pagina={pagina} irA={setPagina} />
        <main className="contenido">
          {pagina === "inicio" && (
            <Inicio estado={estado} error={error} modelos={modelos} irABiblioteca={() => setPagina("biblioteca")}
              ultima={conversaciones?.[0] ?? null} continuar={continuar} />
          )}
          {pagina === "biblioteca" && (
            <Biblioteca estado={estado} modelos={modelos} recargar={recargarModelos} instalaciones={instalaciones}
              irAExplorar={() => setPagina("explorar")} />
          )}
          {/* Explorar is also always mounted, so the search is not lost when changing screens. */}
          <div hidden={pagina !== "explorar"}>
            <Explorar instalaciones={instalaciones} visible={pagina === "explorar"}
              modelosInstalados={modelos?.length ?? 0} />
          </div>
          {/* The chat is always mounted (hidden when not in view): this way the conversation is not lost when changing
              screens, and an answer in progress is not cut. */}
          <div className="pantalla-chat" hidden={pagina !== "chat"}>
            <Chat estado={estado} modelos={modelos} irABiblioteca={() => setPagina("biblioteca")}
              conversaciones={conversaciones} recargarConversaciones={recargarConversaciones}
              abierta={abierta} abrir={(id) => !ocupado && setAbierta(id)} alCambiarOcupado={setOcupado}
              pedidoNueva={pedidoNueva} />
          </div>
          {pagina === "ajustes" && (
            <Ajustes
              ocupado={
                ocupado ||
                (estado?.inUse ?? 0) > 0 ||
                estado?.state === "loading" ||
                Object.values(instalaciones.estado).some((i) => !i.error && !i.hecho)
              }
            />
          )}
        </main>
      </div>
      <BarraDeEstado estado={estado} error={error} />
    </div>
  );
}

// ─── Sidebar ──────────────────────────────────────────────────────────────────

/** Focuses an element by its id, as soon as it is in view (after changing screens). */
export function enfocar(id: string) {
  requestAnimationFrame(() => {
    const el = document.getElementById(id);
    if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) {
      el.focus();
      el.select();
    }
  });
}

const ENTRADAS: { id: Pagina; nombre: string; icono: ReactNode }[] = [
  { id: "inicio", nombre: "Inicio", icono: <path d="M3 11l9-7 9 7M5 10v10h14V10" /> },
  { id: "chat", nombre: "Chat", icono: <path d="M4 5h16v11H9l-5 4V5z" /> },
  { id: "biblioteca", nombre: "Biblioteca", icono: <path d="M12 3l9 5-9 5-9-5 9-5zM3 13l9 5 9-5" /> },
  {
    id: "explorar",
    nombre: "Explorar",
    icono: (
      <>
        <circle cx="11" cy="11" r="7" />
        <path d="M20 20l-4-4" />
      </>
    ),
  },
];

function BarraLateral({ pagina, irA }: { pagina: Pagina; irA: (p: Pagina) => void }) {
  const boton = (id: Pagina, nombre: string, icono: ReactNode, secundario = false) => {
    const atajo = ATAJOS_PANTALLA.find((a) => a.pagina === id)?.tecla;
    return (
      <button
        key={id}
        type="button"
        className={`entrada${pagina === id ? " activa" : ""}${secundario ? " secundaria" : ""}`}
        aria-current={pagina === id ? "page" : undefined}
        title={atajo ? `${nombre} (Ctrl+${atajo})` : nombre}
        aria-keyshortcuts={atajo ? `Control+${atajo}` : undefined}
        onClick={() => irA(id)}
      >
        <Icono>{icono}</Icono>
        {nombre}
      </button>
    );
  };
  return (
    <nav className="lateral" aria-label="Navegación principal">
      <picture className="logo">
        <source srcSet={logoOscuro} media="(prefers-color-scheme: dark)" />
        <img src={logoClaro} alt="Locall." />
      </picture>
      {ENTRADAS.map((e) => boton(e.id, e.nombre, e.icono))}
      <div className="hueco" />
      {boton(
        "ajustes",
        "Ajustes",
        <>
          <circle cx="12" cy="12" r="3" />
          <path d="M12 2v3M12 19v3M4.2 4.2l2.1 2.1M17.7 17.7l2.1 2.1M2 12h3M19 12h3M4.2 19.8l2.1-2.1M17.7 6.3l2.1-2.1" />
        </>,
        true,
      )}
    </nav>
  );
}

// ─── Status bar ───────────────────────────────────────────────────────────────

function BarraDeEstado({ estado, error }: { estado: EstadoLocall | null; error: string | null }) {
  let punto = "apagado";
  let texto = "Abriendo Locall…";
  if (error && !estado) {
    punto = "error";
    texto = "Sin conexión con Locall";
  } else if (estado) {
    if (estado.state === "loading") {
      punto = "ocupado";
      texto = `Cargando ${estado.model ?? ""}…`;
    } else if (estado.state === "ready") {
      punto = estado.inUse > 0 ? "ocupado" : "listo";
      texto = estado.inUse > 0 ? "Generando" : "Motor listo";
    } else {
      texto = "Ningún modelo cargado";
    }
  }
  const total = estado?.memoryTotal ?? 0;
  const modelo = estado?.modelMemory ?? 0;
  const usado = estado ? total - estado.memoryAvailable : 0;
  const pct = (b: number) => (total > 0 ? (b / total) * 100 : 0);
  return (
    <footer className="estado" aria-live="polite">
      <span className="con-punto"><span className={`punto ${punto}`} />{texto}</span>
      {estado?.model && <span>{estado.model}</span>}
      {estado && (
        <span className="ram">
          RAM
          <span className="ram-barra" aria-hidden="true">
            <span className="ram-sistema" style={{ width: `${pct(usado - modelo)}%` }} />
            {modelo > 0 && <span className="ram-modelo" style={{ width: `${pct(modelo)}%` }} />}
          </span>
          <span className="mono">{tamano(usado)} / {tamano(total)}</span>
        </span>
      )}
      {estado?.unloadIn != null && <span>Se libera en {minutosSegundos(estado.unloadIn)}</span>}
      <span className="hueco" />
      {estado && <span className="mono tenue">Locall {estado.version} · {estado.engine}</span>}
    </footer>
  );
}
