# ADR 0021 · Teclado, accesibilidad y pulido de la app

- **Estado:** aceptada
- **Fecha:** 2026-11-10

## Decisión

### Atajos de teclado

- **Ctrl+N** nueva conversación, **Ctrl+K** buscar en Explorar, **Ctrl+1…4** Inicio, Chat, Biblioteca y Explorar,
  **Ctrl+,** Ajustes. Siguen **Enter** (enviar) y **Esc** (parar la respuesta).
- La lista vive en `ui/src/atajos.ts` y la pantalla de Ajustes la muestra tal cual: lo que se enseña es lo que
  funciona. También se reconocen por la posición de la tecla (`e.code`), por si la distribución del teclado da otro
  carácter.
- En la versión final se bloquean los atajos del navegador que aquí solo hacen daño: **F5 / Ctrl+R** (recargar
  borraría lo que hay en pantalla), Ctrl+P, Ctrl+F y Ctrl+G.

### Accesibilidad, con números

- Contrastes medidos con la fórmula de WCAG 2.x en los dos temas. Fallaban dos cosas y se corrigieron sin cambiar el
  verde salvia de la marca:
  - el **texto verde** sobre el fondo verde suave del tema claro (4,31:1): nuevo color `--acento-texto` `#526B5B`
    (4,92:1). El salvia `#5B7464` se sigue usando en fondos, bordes y foco;
  - el **borde de los campos donde se escribe** (1,4:1 y 1,6:1; WCAG 1.4.11 pide 3:1): nuevo `--borde-control`
    `#8F8F8F` (claro, 3,23:1) y `#6B6B6B` (oscuro, 3,49:1).
- Comprobado con pulsaciones reales de Tab que todos los elementos del chat muestran el foco.
- Mientras el modelo escribe, la lista de mensajes lleva `aria-busy`: el lector de pantalla lee la respuesta al
  terminar, en vez de anunciar cada trozo.
- Con la ventana estrecha (menos de 1100 px), el panel de ajustes del chat pasa a abrirse con un botón, por encima de
  la conversación, en vez de dejarla en 180 px.

### Copiar

- Botón **Copiar** en cada respuesta y en cada bloque de código. **Copia la parte Rust** (`SetClipboardData` de
  Windows, en `locall-system`, donde vive todo el código `unsafe`), no el navegador integrado: el portapapeles del
  navegador depende de permisos que no se pueden comprobar desde fuera y que en las pruebas estaban bloqueados. Solo
  se escribe en el portapapeles, nunca se lee.
- No añade crates: `windows-sys` ya estaba en el proyecto (se activan dos módulos más).

### La ventana recuerda su tamaño y posición

- Se guardan al cerrar en `app-ventana.txt` (carpeta de datos) y se recuperan al abrir **solo si la barra de título cae
  dentro de una pantalla conectada**. La ventana arranca oculta y se muestra ya colocada (sin saltos).
- Sin el complemento `tauri-plugin-window-state`: son ~100 líneas propias y probadas, y un crate menos.

## Consecuencias

- La revisión visual se hizo en una vista previa con datos de ejemplo (sin Tauri), en claro y oscuro, a 1280×800 y al
  tamaño mínimo (900×600). El portapapeles y la ventana se comprueban en la app real.
- La prueba del portapapeles está marcada `#[ignore]` porque escribe en el portapapeles de quien la ejecuta.
