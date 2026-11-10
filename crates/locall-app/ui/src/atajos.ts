// Atajos de teclado de la app. La lista está aquí para que la pantalla de Ajustes muestre exactamente los mismos que
// funcionan.

/** Una pantalla de la app. */
export type Pagina = "inicio" | "chat" | "biblioteca" | "explorar" | "ajustes";

/** Ctrl + tecla → pantalla. */
export const ATAJOS_PANTALLA: { tecla: string; pagina: Pagina; nombre: string }[] = [
  { tecla: "1", pagina: "inicio", nombre: "Inicio" },
  { tecla: "2", pagina: "chat", nombre: "Chat" },
  { tecla: "3", pagina: "biblioteca", nombre: "Biblioteca" },
  { tecla: "4", pagina: "explorar", nombre: "Explorar" },
  { tecla: ",", pagina: "ajustes", nombre: "Ajustes" },
];

/** Todos los atajos, para mostrarlos. */
export const LISTA_DE_ATAJOS: { teclas: string; que: string }[] = [
  { teclas: "Ctrl+N", que: "Nueva conversación" },
  { teclas: "Ctrl+K", que: "Buscar modelos en Explorar" },
  ...ATAJOS_PANTALLA.map((a) => ({ teclas: `Ctrl+${a.tecla}`, que: `Ir a ${a.nombre}` })),
  { teclas: "Enter", que: "Enviar el mensaje (Mayús+Enter: salto de línea)" },
  { teclas: "Esc", que: "Parar la respuesta" },
];

/**
 * Atajos del navegador que en una app de escritorio solo hacen daño: recargar (F5, Ctrl+R) borraría lo que hay en
 * pantalla, e imprimir (Ctrl+P) o buscar en la página (Ctrl+F) no tienen sentido aquí. Solo se bloquean en la versión
 * final: en desarrollo, recargar es útil.
 */
export function esAtajoDelNavegador(e: KeyboardEvent): boolean {
  const tecla = e.key.toLowerCase();
  return tecla === "f5" || (e.ctrlKey && (tecla === "r" || tecla === "p" || tecla === "f" || tecla === "g"));
}
