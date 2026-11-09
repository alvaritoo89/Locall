import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// La interfaz vive en `ui/`; el resultado va a `dist/`, que es lo que Tauri mete dentro de la app.
export default defineConfig({
  root: "ui",
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true, // Tauri espera la interfaz exactamente aquí (tauri.conf.json → devUrl)
    host: "127.0.0.1",
  },
  build: {
    outDir: "../dist",
    emptyOutDir: true,
    target: "es2022",
  },
});
