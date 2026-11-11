# ADR 0021 · Keyboard, accessibility and app polish

- **Status:** accepted
- **Date:** 2026-11-10

## Decision

### Keyboard shortcuts

- **Ctrl+N** new conversation, **Ctrl+K** search in Explore, **Ctrl+1…4** Home, Chat, Library and Explore, **Ctrl+,**
  Settings. **Enter** (send) and **Esc** (stop the answer) remain.
- The list lives in `ui/src/atajos.ts` and the Settings screen shows it as-is: what is shown is what works. Shortcuts
  are also recognized by key position (`e.code`), in case the keyboard layout produces another character.
- In the release build, browser shortcuts that only do harm here are blocked: **F5 / Ctrl+R** (reloading would wipe
  what is on screen), Ctrl+P, Ctrl+F and Ctrl+G.

### Accessibility, with numbers

- Contrast measured with the WCAG 2.x formula in both themes. Two things failed and were fixed without changing the
  brand's sage green:
  - the **green text** on the light theme's soft green background (4.31:1): new `--acento-texto` color `#526B5B`
    (4.92:1). The sage `#5B7464` is still used for backgrounds, borders and focus;
  - the **border of input fields** (1.4:1 and 1.6:1; WCAG 1.4.11 asks for 3:1): new `--borde-control` `#8F8F8F`
    (light, 3.23:1) and `#6B6B6B` (dark, 3.49:1).
- Checked with real Tab presses that every element in the chat shows focus.
- While the model is writing, the message list carries `aria-busy`: the screen reader reads the answer once it is
  finished instead of announcing every chunk.
- In a narrow window (under 1100 px), the chat settings panel opens with a button, on top of the conversation, instead
  of squeezing it to 180 px.

### Copy

- A **Copy** button on every answer and every code block. **The copying is done by the Rust side** (Windows
  `SetClipboardData`, in `locall-system`, where all the `unsafe` code lives), not by the embedded browser: the browser
  clipboard depends on permissions that cannot be checked from outside and that were blocked during testing. The
  clipboard is only written, never read.
- No crates added: `windows-sys` was already in the project (two more modules are enabled).

### The window remembers its size and position

- They are saved on close in `app-ventana.txt` (data folder) and restored on open **only if the title bar falls within
  a connected screen**. The window starts hidden and is shown already in place (no jumps).
- Without the `tauri-plugin-window-state` plugin: ~100 lines of our own, tested, and one crate fewer.

## Consequences

- The visual review was done in a preview with sample data (without Tauri), in light and dark themes, at 1280×800 and at
  the minimum size (900×600). The clipboard and the window are checked in the real app.
- The clipboard test is marked `#[ignore]` because it writes to the clipboard of whoever runs it.
