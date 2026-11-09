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

// ─── Conversaciones guardadas ─────────────────────────────────────────────────

/** Una conversación guardada (sin los mensajes). Las fechas, en segundos desde 1970. */
export interface Conversacion {
  id: number;
  title: string;
  /** El último modelo usado (puede haberse eliminado después). */
  model: string | null;
  system: string;
  temperature: number | null;
  created_at: number;
  updated_at: number;
  message_count: number;
}

/** Un mensaje guardado. */
export interface MensajeGuardado {
  index: number;
  role: "user" | "assistant";
  content: string;
  reasoning: string;
  model: string | null;
  completion_tokens: number | null;
  generation_per_second: number | null;
  /** La respuesta se paró antes de terminar. */
  truncated: boolean;
  created_at: number;
}

export const pedirConversaciones = () => invoke<Conversacion[]>("conversaciones");
export const abrirConversacion = (id: number) =>
  invoke<Conversacion & { messages: MensajeGuardado[] }>("abrir_conversacion", { id });
export const crearConversacion = (titulo: string) => invoke<Conversacion>("crear_conversacion", { titulo });
export const renombrarConversacion = (id: number, titulo: string) =>
  invoke<Conversacion>("renombrar_conversacion", { id, titulo });
export const eliminarConversacion = (id: number) => invoke<void>("eliminar_conversacion", { id });

// ─── Conversar ────────────────────────────────────────────────────────────────

/** Lo que llega mientras el modelo responde. */
export type Trozo =
  | { type: "text"; text: string }
  | { type: "reasoning"; text: string }
  | { type: "prompt_progress"; processed: number; total: number };

/** El resumen al terminar una respuesta. */
export interface FinRespuesta {
  finish: "stop" | "length" | "cancelled";
  usage?: { prompt_tokens: number; cached_tokens: number; completion_tokens: number };
  speed?: { prompt_per_second: number | null; generation_per_second: number | null };
  /** Lo que Locall guardó (no llega si se paró: Locall lo guarda igualmente). */
  conversation?: Conversacion;
  message?: MensajeGuardado | null;
}

/** Lo que se envía al conversar. */
export interface Envio {
  /** Identifica la respuesta, para poder pararla. */
  peticion: string;
  conversacion: number;
  contenido: string;
  modelo: string;
  sistema: string;
  temperatura: number | null;
}

/**
 * Escribe un mensaje en una conversación guardada; `alRecibir` recibe cada trozo de la respuesta. Locall envía al
 * modelo la conversación entera y guarda la pregunta y la respuesta.
 */
export function conversar(envio: Envio, alRecibir: (t: Trozo) => void): Promise<FinRespuesta> {
  const canal = new Channel<Trozo>();
  canal.onmessage = alRecibir;
  return invoke<FinRespuesta>("conversar", { ...envio, canal });
}

export const parar = (peticion: string) => invoke<void>("parar", { peticion });

/** Hoy, Ayer, Esta semana o Anteriores, según la fecha (en segundos). */
export function grupoDeFecha(segundos: number, ahora = new Date()): string {
  const dia = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const dias = Math.round((dia(ahora) - dia(new Date(segundos * 1000))) / 86_400_000);
  if (dias <= 0) return "Hoy";
  if (dias === 1) return "Ayer";
  if (dias < 7) return "Esta semana";
  return "Anteriores";
}

/** "hace 5 min", "hace 3 h", "ayer", "12 oct". */
export function haceCuanto(segundos: number, ahora = Date.now()): string {
  const s = Math.max(0, Math.floor(ahora / 1000 - segundos));
  if (s < 60) return "ahora mismo";
  if (s < 3600) return `hace ${Math.floor(s / 60)} min`;
  const grupo = grupoDeFecha(segundos, new Date(ahora));
  if (grupo === "Hoy") return `hace ${Math.floor(s / 3600)} h`;
  if (grupo === "Ayer") return "ayer";
  return new Date(segundos * 1000).toLocaleDateString("es-ES", { day: "numeric", month: "short" });
}

/** `12.43` → `12,4` */
export const decimal = (n: number, decimales = 1) =>
  n.toLocaleString("es-ES", { minimumFractionDigits: decimales, maximumFractionDigits: decimales });
