import { type ReactNode, useEffect, useState } from "react";

import { type EstadoLocall, gb, pedirEstado } from "./locall";
import logoClaro from "./assets/logo-claro.png";
import logoOscuro from "./assets/logo-oscuro.png";

type Pagina = "inicio" | "chat" | "biblioteca" | "explorar" | "ajustes";

/** Cada cuánto se pregunta a Locall cómo está (para la barra de estado). */
const INTERVALO_ESTADO_MS = 2000;

export function App() {
  const [pagina, setPagina] = useState<Pagina>("inicio");
  const [estado, setEstado] = useState<EstadoLocall | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Preguntar el estado al empezar y cada 2 s. (La primera vez puede tardar: si Locall no está abierto, se abre.)
  useEffect(() => {
    let vivo = true;
    const preguntar = () =>
      pedirEstado()
        .then((e) => vivo && (setEstado(e), setError(null)))
        .catch((e) => vivo && setError(String(e)));
    preguntar();
    const temporizador = setInterval(preguntar, INTERVALO_ESTADO_MS);
    return () => {
      vivo = false;
      clearInterval(temporizador);
    };
  }, []);

  return (
    <div className="app">
      <div className="cuerpo">
        <BarraLateral pagina={pagina} irA={setPagina} />
        <main className="contenido">
          {pagina === "inicio" ? (
            <Inicio estado={estado} error={error} />
          ) : (
            <Proximamente pagina={pagina} />
          )}
        </main>
      </div>
      <BarraDeEstado estado={estado} error={error} />
    </div>
  );
}

// ─── Barra lateral ────────────────────────────────────────────────────────────

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

function Icono({ children }: { children: ReactNode }) {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8"
      strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      {children}
    </svg>
  );
}

function BarraLateral({ pagina, irA }: { pagina: Pagina; irA: (p: Pagina) => void }) {
  const boton = (id: Pagina, nombre: string, icono: ReactNode, secundario = false) => (
    <button
      key={id}
      type="button"
      className={`entrada${pagina === id ? " activa" : ""}${secundario ? " secundaria" : ""}`}
      aria-current={pagina === id ? "page" : undefined}
      onClick={() => irA(id)}
    >
      <Icono>{icono}</Icono>
      {nombre}
    </button>
  );
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

// ─── Inicio ───────────────────────────────────────────────────────────────────

function Inicio({ estado, error }: { estado: EstadoLocall | null; error: string | null }) {
  return (
    <>
      <h1>Inicio</h1>
      {error && !estado && (
        <section className="tarjeta aviso" role="alert">
          <h2>No se puede conectar con Locall</h2>
          <p>{error}</p>
        </section>
      )}
      {!estado && !error && <p className="tenue">Abriendo Locall…</p>}
      {estado && (
        <section className="tarjeta">
          <div className="titulo-tarjeta">
            <h2>Tu equipo</h2>
            <span className="tenue pequeño">Detectado ahora</span>
          </div>
          <div className="datos">
            <Dato nombre="Procesador" valor={estado.cpu ?? "Desconocido"} />
            <Dato nombre="Hilos" valor={String(estado.threads)} />
            <Dato nombre="Memoria" valor={gb(estado.memoryTotal)} />
          </div>
          <ReglaDeMemoria total={estado.memoryTotal} libre={estado.memoryAvailable} />
          {estado.warnings.map((aviso) => (
            <p key={aviso} className="pequeño aviso-texto">Aviso: {aviso}</p>
          ))}
        </section>
      )}
    </>
  );
}

function Dato({ nombre, valor }: { nombre: string; valor: string }) {
  return (
    <div className="dato">
      <span className="tenue pequeño">{nombre}</span>
      <span className="valor">{valor}</span>
    </div>
  );
}

/** La regla de memoria de la maqueta: lo usado y lo libre. (El trozo del modelo cargado llegará en la parte 5b.) */
function ReglaDeMemoria({ total, libre }: { total: number; libre: number }) {
  const usado = total - libre;
  const porcentaje = total > 0 ? (usado / total) * 100 : 0;
  return (
    <div className="regla">
      <div className="regla-barra" role="img"
        aria-label={`Memoria: ${gb(usado)} en uso, ${gb(libre)} libres de ${gb(total)}`}>
        <div className="regla-usado" style={{ width: `${porcentaje}%` }} />
      </div>
      <div className="leyenda pequeño">
        <span><span className="muestra usado" />En uso {gb(usado)}</span>
        <span><span className="muestra libre" />Libre {gb(libre)}</span>
      </div>
    </div>
  );
}

// ─── Lo que llega en las próximas partes ──────────────────────────────────────

const PROXIMAS: Record<Exclude<Pagina, "inicio">, { titulo: string; parte: string }> = {
  chat: { titulo: "Chat", parte: "5c" },
  biblioteca: { titulo: "Biblioteca", parte: "5b" },
  explorar: { titulo: "Explorar", parte: "5e" },
  ajustes: { titulo: "Ajustes", parte: "5e" },
};

function Proximamente({ pagina }: { pagina: Exclude<Pagina, "inicio"> }) {
  const { titulo, parte } = PROXIMAS[pagina];
  return (
    <>
      <h1>{titulo}</h1>
      <p className="tenue">Esta pantalla llegará en la parte {parte} de la Fase 5.</p>
    </>
  );
}

// ─── Barra de estado ──────────────────────────────────────────────────────────

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
      punto = "apagado";
      texto = "Ningún modelo cargado";
    }
  }
  const usado = estado ? estado.memoryTotal - estado.memoryAvailable : 0;
  return (
    <footer className="estado" aria-live="polite">
      <span className="con-punto"><span className={`punto ${punto}`} />{texto}</span>
      {estado?.model && <span>{estado.model}</span>}
      {estado && (
        <span className="ram">
          RAM
          <span className="ram-barra" aria-hidden="true">
            <span style={{ width: `${(usado / estado.memoryTotal) * 100}%` }} />
          </span>
          <span className="mono">{gb(usado)} / {gb(estado.memoryTotal)}</span>
        </span>
      )}
      <span className="hueco" />
      {estado && <span className="mono tenue">Locall {estado.version} · {estado.engine}</span>}
    </footer>
  );
}
