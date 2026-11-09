import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

// Las fuentes van dentro de la app (licencia OFL): nunca se piden a internet.
import "@fontsource-variable/inter";
import "@fontsource-variable/jetbrains-mono";
import "./estilos.css";

import { App } from "./App";

createRoot(document.getElementById("raiz")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
