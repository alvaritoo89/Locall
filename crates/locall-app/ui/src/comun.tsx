// Pieces shared by several screens.

import { type ReactNode, useEffect, useState } from "react";

import { type EstadoLocall, type Modelo, copiarTexto, tamano, textoEncaje } from "./locall";

/** A stroke icon (like the ones in the mockups). */
export function Icono({ children }: { children: ReactNode }) {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8"
      strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      {children}
    </svg>
  );
}

/**
 * The memory bar: what the system and the other apps use, what the loaded model uses (striped, in sage green) and
 * what is free.
 */
export function ReglaDeMemoria({ estado }: { estado: EstadoLocall }) {
  const total = estado.memoryTotal;
  const libre = estado.memoryAvailable;
  const modelo = estado.modelMemory ?? 0;
  const sistema = Math.max(0, total - libre - modelo);
  const pct = (bytes: number) => (total > 0 ? (bytes / total) * 100 : 0);
  const marcas = Math.max(1, Math.round(total / 4e9));
  const descripcion =
    `Memoria: ${tamano(sistema)} en uso por el sistema y otras aplicaciones` +
    (modelo ? `, ${tamano(modelo)} del modelo ${estado.model ?? ""}` : "") +
    `, ${tamano(libre)} libres de ${tamano(total)}`;
  return (
    <div className="regla">
      <div className="regla-barra" role="img" aria-label={descripcion}>
        <div className="regla-usado" style={{ width: `${pct(sistema)}%` }} />
        {modelo > 0 && <div className="regla-modelo" style={{ width: `${pct(modelo)}%` }} />}
      </div>
      <div className="regla-marcas mono pequeño tenue" aria-hidden="true">
        {Array.from({ length: marcas + 1 }, (_, i) => (
          <span key={i}>{i === marcas ? tamano(total) : Math.round((total / marcas) * i / 1e9)}</span>
        ))}
      </div>
      <div className="leyenda pequeño">
        <span><span className="muestra usado" />Sistema y apps {tamano(sistema)}</span>
        {modelo > 0 && <span><span className="muestra modelo" />{estado.model} {tamano(modelo)}</span>}
        <span><span className="muestra libre" />Libre {tamano(libre)}</span>
      </div>
    </div>
  );
}

/** A model's state in the tables: loaded, or whether it fits on the computer. */
export function EstadoDelModelo({ modelo, estado }: { modelo: Modelo; estado: EstadoLocall | null }) {
  if (estado?.model === modelo.id && estado.state !== "idle") {
    return (
      <span className="cargado">
        <span className="punto ocupado" />
        {estado.state === "loading" ? "Cargando…" : "Cargado"}
      </span>
    );
  }
  if (modelo.compatible === false) {
    return <span className="tono-error">El motor no lo admite</span>;
  }
  const { texto, tono } = textoEncaje(modelo.fit);
  const detalle = modelo.memory_estimate_bytes
    ? `Necesita unos ${tamano(modelo.memory_estimate_bytes)} de memoria`
    : undefined;
  return (
    <span className={`tono-${tono}`} title={detalle}>
      {texto}
    </span>
  );
}

/** A model's name with its quantization, as in the mockups. */
export function NombreDelModelo({ modelo }: { modelo: Modelo }) {
  return (
    <span className="nombre-modelo">
      {modelo.id}
      {modelo.quantization && <span className="mono pequeño tenue"> {modelo.quantization}</span>}
    </span>
  );
}

/**
 * A button that copies `texto` to the clipboard and says «Copiado» for a moment. (The clipboard is only used when it
 * is pressed: the app never reads it.)
 */
export function BotonCopiar({ texto, etiqueta, clase = "" }: { texto: string; etiqueta: string; clase?: string }) {
  const [estado, setEstado] = useState<"listo" | "copiado" | "error">("listo");
  useEffect(() => {
    if (estado === "listo") return;
    const t = setTimeout(() => setEstado("listo"), 1500);
    return () => clearTimeout(t);
  }, [estado]);
  const copiar = () =>
    copiarTexto(texto).then(
      () => setEstado("copiado"),
      () => setEstado("error"),
    );
  return (
    <button type="button" className={`copiar ${clase}`} onClick={copiar} aria-label={etiqueta}>
      {estado === "copiado" ? "Copiado" : estado === "error" ? "No se pudo copiar" : "Copiar"}
      <span className="solo-lectores" role="status">{estado === "copiado" ? "Copiado al portapapeles" : ""}</span>
    </button>
  );
}
