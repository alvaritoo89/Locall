// The app's keyboard shortcuts. The list lives here so the Settings screen shows exactly the same ones that work.

/** A screen of the app. */
export type Pagina = "inicio" | "chat" | "biblioteca" | "explorar" | "ajustes";

/** Ctrl + key → screen. */
export const ATAJOS_PANTALLA: { tecla: string; pagina: Pagina; nombre: string }[] = [
  { tecla: "1", pagina: "inicio", nombre: "Inicio" },
  { tecla: "2", pagina: "chat", nombre: "Chat" },
  { tecla: "3", pagina: "biblioteca", nombre: "Biblioteca" },
  { tecla: "4", pagina: "explorar", nombre: "Explorar" },
  { tecla: ",", pagina: "ajustes", nombre: "Ajustes" },
];

/** All the shortcuts, to display them. */
export const LISTA_DE_ATAJOS: { teclas: string; que: string }[] = [
  { teclas: "Ctrl+N", que: "Nueva conversación" },
  { teclas: "Ctrl+K", que: "Buscar modelos en Explorar" },
  ...ATAJOS_PANTALLA.map((a) => ({ teclas: `Ctrl+${a.tecla}`, que: `Ir a ${a.nombre}` })),
  { teclas: "Enter", que: "Enviar el mensaje (Mayús+Enter: salto de línea)" },
  { teclas: "Esc", que: "Parar la respuesta" },
];

/**
 * Browser shortcuts that only do harm in a desktop app: reloading (F5, Ctrl+R) would wipe what is on screen, and
 * printing (Ctrl+P) or finding in the page (Ctrl+F) make no sense here. They are only blocked in the release build:
 * in development, reloading is useful.
 */
export function esAtajoDelNavegador(e: KeyboardEvent): boolean {
  const tecla = e.key.toLowerCase();
  return tecla === "f5" || (e.ctrlKey && (tecla === "r" || tecla === "p" || tecla === "f" || tecla === "g"));
}
