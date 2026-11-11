import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

// The fonts are bundled with the app (OFL license): they are never requested from the internet.
import "@fontsource-variable/inter";
import "@fontsource-variable/jetbrains-mono";
import "./estilos.css";

import { App } from "./App";

createRoot(document.getElementById("raiz")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
