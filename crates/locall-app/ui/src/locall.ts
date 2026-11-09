// Lo que la interfaz pide a la parte Rust de la app (que es quien habla con Locall). La interfaz nunca ve el token.

import { invoke } from "@tauri-apps/api/core";

/** Estado de Locall y del equipo (orden `estado` de src/main.rs). */
export interface EstadoLocall {
  version: string;
  engine: string;
  /** "idle" (sin modelo), "loading" (cargando) o "ready" (listo). */
  state: "idle" | "loading" | "ready";
  model: string | null;
  inUse: number;
  warnings: string[];
  cpu: string | null;
  threads: number;
  memoryTotal: number;
  memoryAvailable: number;
}

export function pedirEstado(): Promise<EstadoLocall> {
  return invoke<EstadoLocall>("estado");
}

/** `16_800_000_000` → `16,8 GB` (unidades decimales, como en la CLI). */
export function gb(bytes: number): string {
  return `${(bytes / 1e9).toLocaleString("es-ES", { maximumFractionDigits: 1, minimumFractionDigits: 1 })} GB`;
}
