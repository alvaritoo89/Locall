// What the UI asks of the app's Rust side (which is what talks to Locall). The UI never sees the token.

import { Channel, invoke } from "@tauri-apps/api/core";

/** State of Locall and of the computer (the `estado` command in src/main.rs). */
export interface EstadoLocall {
  version: string;
  engine: string;
  /** "idle" (no model), "loading" or "ready". */
  state: "idle" | "loading" | "ready";
  model: string | null;
  inUse: number;
  /** Memory the loaded model takes right now. */
  modelMemory: number | null;
  /** Seconds left until the model unloads due to inactivity. */
  unloadIn: number | null;
  warnings: string[];
  cpu: string | null;
  threads: number;
  memoryTotal: number;
  memoryAvailable: number;
}

/** Does it fit on the computer? (`locall-compat`) */
export type Encaje = "comfortable" | "fits" | "tight" | "too_big";

/** An installed model (as the API gives it). */
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

/** A Hugging Face repository and its GGUFs. */
export interface Repositorio {
  repo: string;
  commit: string;
  license: string | null;
  gated: boolean;
  files: { path: string; size_bytes: number; sha256: string | null }[];
}

/** A progress notice of an installation. */
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
/** A model from Locall's catalog. */
export interface ModeloCatalogo {
  name: string;
  title: string;
  publisher: string;
  description: string;
  tags: string[];
  license: string;
  context_length: number;
  /** Always «compatible»: the engine recognizes its format (they have not been tested one by one). */
  status: "compatible";
  repo: string;
  size_bytes: number;
  installed: boolean;
  /** Approximate: from the file size and the memory free right now. */
  fit_estimate: Encaje;
  memory_estimate_bytes: number;
}

/** Locall's catalog. */
export interface Catalogo {
  published: number;
  /** «builtin»: the one shipped with the app; «downloaded»: a newer one from GitHub. */
  source: "builtin" | "downloaded";
  checked_at: number | null;
  models: ModeloCatalogo[];
  /** If it could not check whether there was a newer one. */
  warning?: string;
}

export const pedirCatalogo = () => invoke<Catalogo>("catalogo");

/** Installs a catalog model by its name. */
export function instalarDelCatalogo(nombre: string, alProgresar: (p: Progreso) => void): Promise<Modelo> {
  const canal = new Channel<Progreso>();
  canal.onmessage = alProgresar;
  return invoke<Modelo>("instalar_catalogo", { nombre, canal });
}

/** A repository found when searching Hugging Face. */
export interface Encontrado {
  repo: string;
  downloads: number;
  likes: number;
  license: string | null;
  /** What it is for according to Hugging Face (`text-generation`…). */
  task: string | null;
}

export const buscarModelos = (texto: string) => invoke<Encontrado[]>("buscar_modelos", { texto });

/** `26265` → `26 mil`; `1_200_000` → `1,2 M` */
export function cifraCorta(n: number): string {
  if (n >= 1_000_000) return `${decimal(n / 1_000_000)} M`;
  if (n >= 1000) return `${Math.round(n / 1000)} mil`;
  return String(n);
}

/** A set of settings (those in use or those saved in `config.toml`). */
export interface ValoresAjustes {
  context: number;
  threads: number | "auto";
  unload_minutes: number;
  exit_minutes: number;
  log_days: number;
  models_folder: string;
  port: number;
  parallel: number;
}

/** What `GET /api/settings` returns. */
export interface Ajustes {
  active: ValoresAjustes;
  saved: ValoresAjustes;
  /** What is saved differs from what is in use: Locall must be restarted. */
  restart_needed: boolean;
  threads_available: number;
  data_folder: string;
}

export const pedirAjustes = () => invoke<Ajustes>("ajustes");
export const guardarAjustes = (cambios: Record<string, number | "auto">) =>
  invoke<Ajustes>("guardar_ajustes", { cambios });
export const reiniciarLocall = () => invoke<void>("reiniciar_locall");
export const abrirCarpeta = (cual: "datos" | "modelos") => invoke<void>("abrir_carpeta", { cual });

export const cancelarInstalacion = (repo: string, archivo: string) =>
  invoke<void>("cancelar_instalacion", { repo, archivo });

/**
 * Imports a `.gguf` from the computer: opens the Windows dialog to pick it. `alElegir` receives the file name as soon
 * as it is picked. Returns the imported model, or `null` if the dialog was closed without picking.
 */
export function importarGguf(alElegir: (nombre: string) => void): Promise<Modelo | null> {
  const canal = new Channel<string>();
  canal.onmessage = alElegir;
  return invoke<Modelo | null>("importar_gguf", { canal });
}

/** Installs a model; `alProgresar` receives each progress notice. Returns the installed model. */
export function instalarModelo(
  repo: string,
  archivo: string,
  alProgresar: (p: Progreso) => void,
): Promise<Modelo> {
  const canal = new Channel<Progreso>();
  canal.onmessage = alProgresar;
  return invoke<Modelo>("instalar", { repo, archivo, canal });
}

// ─── Formats ──────────────────────────────────────────────────────────────────

/** `1_185_376` → `1,2 MB` (decimal units, like the CLI and Hugging Face). */
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

/** Text and tone for "does it fit?". */
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

/** An `invoke` error as text. */
export const textoError = (e: unknown) => (typeof e === "string" ? e : String(e));

// ─── Saved conversations ──────────────────────────────────────────────────────

/** A saved conversation (without the messages). Dates in seconds since 1970. */
export interface Conversacion {
  id: number;
  title: string;
  /** The last model used (it may have been removed since). */
  model: string | null;
  system: string;
  temperature: number | null;
  created_at: number;
  updated_at: number;
  message_count: number;
}

/** A saved message. */
export interface MensajeGuardado {
  index: number;
  role: "user" | "assistant";
  content: string;
  reasoning: string;
  model: string | null;
  completion_tokens: number | null;
  generation_per_second: number | null;
  /** The answer was stopped before it finished. */
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

// ─── Chatting ─────────────────────────────────────────────────────────────────

/** What arrives while the model answers. */
export type Trozo =
  | { type: "text"; text: string }
  | { type: "reasoning"; text: string }
  | { type: "prompt_progress"; processed: number; total: number };

/** The summary when an answer ends. */
export interface FinRespuesta {
  finish: "stop" | "length" | "cancelled";
  usage?: { prompt_tokens: number; cached_tokens: number; completion_tokens: number };
  speed?: { prompt_per_second: number | null; generation_per_second: number | null };
  /** What Locall saved (it does not arrive if it was stopped: Locall saves it anyway). */
  conversation?: Conversacion;
  message?: MensajeGuardado | null;
}

/** What is sent when chatting. */
export interface Envio {
  /** Identifies the answer, so it can be stopped. */
  peticion: string;
  conversacion: number;
  contenido: string;
  modelo: string;
  sistema: string;
  temperatura: number | null;
}

/**
 * Writes a message to a saved conversation; `alRecibir` receives each chunk of the answer. Locall sends the whole
 * conversation to the model and saves the question and the answer.
 */
export function conversar(envio: Envio, alRecibir: (t: Trozo) => void): Promise<FinRespuesta> {
  const canal = new Channel<Trozo>();
  canal.onmessage = alRecibir;
  return invoke<FinRespuesta>("conversar", { ...envio, canal });
}

export const parar = (peticion: string) => invoke<void>("parar", { peticion });

/** Copies a text to the clipboard (done by the Rust side: see `copiar` in src/main.rs). */
export const copiarTexto = (texto: string) => invoke<void>("copiar", { texto });

/** Hoy, Ayer, Esta semana or Anteriores (Today, Yesterday, This week, Earlier), by date (in seconds). */
export function grupoDeFecha(segundos: number, ahora = new Date()): string {
  const dia = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const dias = Math.round((dia(ahora) - dia(new Date(segundos * 1000))) / 86_400_000);
  if (dias <= 0) return "Hoy";
  if (dias === 1) return "Ayer";
  if (dias < 7) return "Esta semana";
  return "Anteriores";
}

/** "hace 5 min", "hace 3 h", "ayer", "12 oct" (relative time, in Spanish). */
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
