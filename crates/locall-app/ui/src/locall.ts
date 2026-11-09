// Lo que la interfaz pide a la parte Rust de la app (que es quien habla con Locall). La interfaz nunca ve el token.

import { Channel, invoke } from "@tauri-apps/api/core";

/** Estado de Locall y del equipo (orden `estado` de src/main.rs). */
export interface EstadoLocall {
  version: string;
  engine: string;
  /** "idle" (sin modelo), "loading" (cargando) o "ready" (listo). */
  state: "idle" | "loading" | "ready";
  model: string | null;
  inUse: number;
  /** Memoria que ocupa ahora el modelo cargado. */
  modelMemory: number | null;
  /** Segundos que faltan para que el modelo se descargue por inactividad. */
  unloadIn: number | null;
  warnings: string[];
  cpu: string | null;
  threads: number;
  memoryTotal: number;
  memoryAvailable: number;
}

/** ¿Cabe en el equipo? (`locall-compat`) */
export type Encaje = "comfortable" | "fits" | "tight" | "too_big";

/** Un modelo instalado (como lo da la API). */
export interface Modelo {
  id: string;
  name: string;
  source: string;
  revision: string;
  quantization: string | null;
  license: string | null;
  architecture: string | null;
  context_max: number | null;
  size_bytes: number;
  installed_at: number;
  last_used_at: number | null;
  fit: Encaje | null;
  memory_estimate_bytes: number | null;
  compatible: boolean | null;
}

/** Un repositorio de Hugging Face y sus GGUF. */
export interface Repositorio {
  repo: string;
  commit: string;
  license: string | null;
  gated: boolean;
  files: { path: string; size_bytes: number; sha256: string | null }[];
}

/** Un aviso de progreso de una instalación. */
export interface Progreso {
  part: number;
  parts: number;
  downloaded_bytes: number;
  total_bytes: number;
}

export const pedirEstado = () => invoke<EstadoLocall>("estado");
export const pedirModelos = () => invoke<Modelo[]>("modelos");
export const eliminarModelo = (id: string) => invoke<number>("eliminar", { id });
export const consultarRepo = (repo: string) => invoke<Repositorio>("consultar_repo", { repo });
export const cancelarInstalacion = (repo: string, archivo: string) =>
  invoke<void>("cancelar_instalacion", { repo, archivo });

/** Instala un modelo; `alProgresar` recibe cada aviso de progreso. Devuelve el modelo instalado. */
export function instalarModelo(
  repo: string,
  archivo: string,
  alProgresar: (p: Progreso) => void,
): Promise<Modelo> {
  const canal = new Channel<Progreso>();
  canal.onmessage = alProgresar;
  return invoke<Modelo>("instalar", { repo, archivo, canal });
}

// ─── Formatos ─────────────────────────────────────────────────────────────────

/** `1_185_376` → `1,2 MB` (unidades decimales, como la CLI y Hugging Face). */
export function tamano(bytes: number): string {
  const unidades = ["B", "KB", "MB", "GB", "TB"];
  let valor = bytes;
  let i = 0;
  while (valor >= 1000 && i < unidades.length - 1) {
    valor /= 1000;
    i++;
  }
  return i === 0
    ? `${bytes} B`
    : `${valor.toLocaleString("es-ES", { minimumFractionDigits: 1, maximumFractionDigits: 1 })} ${unidades[i]}`;
}

/** `252` → `4:12` */
export function minutosSegundos(segundos: number): string {
  const m = Math.floor(segundos / 60);
  const s = Math.floor(segundos % 60);
  return `${m}:${String(s).padStart(2, "0")}`;
}

/** Texto y tono para "¿cabe?". */
export function textoEncaje(encaje: Encaje | null): { texto: string; tono: "normal" | "aviso" | "error" } {
  switch (encaje) {
    case "comfortable":
      return { texto: "Cabe de sobra", tono: "normal" };
    case "fits":
      return { texto: "Cabe", tono: "normal" };
    case "tight":
      return { texto: "En el límite", tono: "aviso" };
    case "too_big":
      return { texto: "No cabe", tono: "error" };
    default:
      return { texto: "—", tono: "normal" };
  }
}

/** Un error de `invoke` como texto. */
export const textoError = (e: unknown) => (typeof e === "string" ? e : String(e));
