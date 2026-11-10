// Formato de las respuestas: bloques de código (```), `código en línea` y **negrita**.
//
// El texto lo escribe el modelo: NO es de fiar. Por eso no se convierte a HTML (nada de innerHTML ni de librerías que
// generen HTML): se construyen elementos de React, que siempre tratan el texto como texto. Si un modelo escribe
// "<script>", se ve tal cual y no se ejecuta.

import type { ReactNode } from "react";

import { BotonCopiar } from "./comun";

/** Un trozo del texto: un bloque de código o un párrafo normal. */
type Bloque = { tipo: "codigo"; lenguaje: string; texto: string } | { tipo: "parrafo"; texto: string };

/** Divide el texto en bloques de código (```lenguaje … ```) y párrafos (separados por líneas en blanco). */
export function dividir(texto: string): Bloque[] {
  const bloques: Bloque[] = [];
  const partes = texto.split(/```/);
  partes.forEach((parte, i) => {
    if (i % 2 === 1) {
      // Dentro de un bloque de código: la primera línea puede ser el lenguaje.
      const salto = parte.indexOf("\n");
      const primera = salto === -1 ? parte : parte.slice(0, salto);
      const esLenguaje = /^[\w+#.-]{0,20}$/.test(primera.trim());
      bloques.push({
        tipo: "codigo",
        lenguaje: esLenguaje ? primera.trim() : "",
        texto: (esLenguaje && salto !== -1 ? parte.slice(salto + 1) : parte).replace(/\n$/, ""),
      });
    } else {
      for (const parrafo of parte.split(/\n{2,}/)) {
        if (parrafo.trim()) bloques.push({ tipo: "parrafo", texto: parrafo.replace(/^\n+|\n+$/g, "") });
      }
    }
  });
  return bloques;
}

/** Dentro de un párrafo: `código` y **negrita**; los saltos de línea se respetan. */
function enLinea(texto: string): ReactNode[] {
  const resultado: ReactNode[] = [];
  const patron = /(`[^`\n]+`|\*\*[^*\n]+\*\*)/g;
  let ultimo = 0;
  let clave = 0;
  for (const coincidencia of texto.matchAll(patron)) {
    const inicio = coincidencia.index ?? 0;
    if (inicio > ultimo) resultado.push(texto.slice(ultimo, inicio));
    const t = coincidencia[0];
    resultado.push(
      t.startsWith("`") ? (
        <code key={clave++} className="codigo-en-linea">{t.slice(1, -1)}</code>
      ) : (
        <strong key={clave++}>{t.slice(2, -2)}</strong>
      ),
    );
    ultimo = inicio + t.length;
  }
  if (ultimo < texto.length) resultado.push(texto.slice(ultimo));
  return resultado;
}

/** Muestra una respuesta con formato. Mientras se escribe, un bloque de código sin cerrar se ve ya como código. */
export function Formato({ texto }: { texto: string }) {
  return (
    <>
      {dividir(texto).map((b, i) =>
        b.tipo === "codigo" ? (
          <div key={i} className="contenedor-codigo">
            <pre className="bloque-codigo" aria-label={b.lenguaje ? `Código ${b.lenguaje}` : "Código"}>
              <code>{b.texto}</code>
            </pre>
            <BotonCopiar texto={b.texto} etiqueta={b.lenguaje ? `Copiar el código ${b.lenguaje}` : "Copiar el código"}
              clase="sobre-codigo" />
          </div>
        ) : (
          <p key={i} className="parrafo-respuesta">{enLinea(b.texto)}</p>
        ),
      )}
    </>
  );
}
