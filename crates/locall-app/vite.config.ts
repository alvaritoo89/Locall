import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The UI lives in `ui/`; the output goes to `dist/`, which is what Tauri puts inside the app.
export default defineConfig({
  root: "ui",
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true, // Tauri expects the UI exactly here (tauri.conf.json → devUrl)
    host: "127.0.0.1",
  },
  build: {
    outDir: "../dist",
    emptyOutDir: true,
    target: "es2022",
  },
});
