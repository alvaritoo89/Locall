// La web funciona sin JavaScript. Esto solo añade: botones «Copiar» en los comandos y los filtros de la biblioteca.
// Sin estadísticas, sin cookies, sin pedir nada a otros sitios.

document.documentElement.classList.add("con-js");

// «Copiar»: el botón está justo después del <code> con el comando.
document.addEventListener("click", (e) => {
  const boton = e.target.closest(".copiar");
  if (!boton) return;
  const codigo = boton.parentElement.querySelector("code");
  if (!codigo) return;
  navigator.clipboard.writeText(codigo.textContent.trim()).then(
    () => avisar(boton, "Copiado"),
    () => avisar(boton, "Selecciónalo y cópialo"),
  );
});

function avisar(boton, texto) {
  const antes = boton.textContent;
  boton.textContent = texto;
  setTimeout(() => (boton.textContent = antes), 1600);
}

// Filtros de la biblioteca: por texto, por uso (etiqueta) y por tamaño. Cada ficha lleva data-texto, data-etiquetas y
// data-gb. Filtrar no envía nada a ningún sitio.
const filtros = document.querySelector(".filtros");
if (filtros) {
  const estado = { etiqueta: "todos", tamano: "todos", texto: "" };
  const cabe = (gb) =>
    estado.tamano === "todos" ||
    (estado.tamano === "pequeno" && gb < 2) ||
    (estado.tamano === "mediano" && gb >= 2 && gb < 6) ||
    (estado.tamano === "grande" && gb >= 6);
  const aplicar = () => {
    let visibles = 0;
    // Solo las fichas del catálogo (no los resultados de Hugging Face).
    for (const ficha of document.querySelectorAll("ul.modelos:not(.lista-hf) > .ficha")) {
      const etiquetas = ficha.dataset.etiquetas.split(" ");
      const ver =
        (estado.etiqueta === "todos" || etiquetas.includes(estado.etiqueta)) &&
        cabe(Number(ficha.dataset.gb)) &&
        estado.texto.split(/\s+/).every((palabra) => ficha.dataset.texto.includes(palabra));
      ficha.hidden = !ver;
      if (ver) visibles++;
    }
    const cuenta = document.querySelector(".cuenta");
    if (cuenta) cuenta.textContent = `${visibles} ${visibles === 1 ? "modelo" : "modelos"}`;
  };
  filtros.addEventListener("click", (e) => {
    const boton = e.target.closest("button[data-filtro]");
    if (!boton) return;
    const grupo = boton.dataset.filtro;
    estado[grupo] = boton.dataset.valor;
    for (const b of filtros.querySelectorAll(`button[data-filtro="${grupo}"]`)) {
      b.setAttribute("aria-pressed", String(b === boton));
    }
    aplicar();
  });

  const buscar = document.getElementById("buscar");
  buscar.addEventListener("input", () => {
    estado.texto = buscar.value.trim().toLowerCase();
    aplicar();
  });
  document.querySelector(".buscador").addEventListener("submit", (e) => {
    e.preventDefault();
    buscarEnHuggingFace(buscar.value.trim());
  });
}

// ─── Buscar en Hugging Face (solo al pulsar el botón) ───

/** Solo nombres de repositorio con la forma normal (propietario/nombre). */
const REPO_VALIDO = /^[A-Za-z0-9][A-Za-z0-9._-]{0,95}\/[A-Za-z0-9][A-Za-z0-9._-]{0,95}$/;

async function buscarEnHuggingFace(texto) {
  const seccion = document.querySelector(".resultados-hf");
  const estado = seccion.querySelector(".estado-hf");
  const lista = seccion.querySelector(".lista-hf");
  if (!texto) return;
  seccion.hidden = false;
  lista.replaceChildren();
  estado.textContent = "Buscando en Hugging Face…";
  const url =
    "https://huggingface.co/api/models?search=" +
    encodeURIComponent(texto.slice(0, 100)) +
    "&filter=gguf&sort=downloads&direction=-1&limit=20";
  let resultados;
  try {
    const respuesta = await fetch(url, { credentials: "omit", referrerPolicy: "no-referrer" });
    if (!respuesta.ok) throw new Error(String(respuesta.status));
    resultados = await respuesta.json();
  } catch {
    estado.textContent = "No se pudo conectar con Hugging Face. Inténtalo de nuevo en un momento.";
    return;
  }
  const validos = (Array.isArray(resultados) ? resultados : []).filter(
    (r) => typeof r.id === "string" && REPO_VALIDO.test(r.id) && r.private !== true,
  );
  estado.textContent = validos.length
    ? `${validos.length} resultados, los más descargados primero.`
    : "No hay modelos GGUF con ese nombre en Hugging Face.";
  // Todo con textContent: lo que llega de fuera nunca se interpreta como HTML.
  for (const r of validos) {
    const licencia = (r.tags || []).find((t) => typeof t === "string" && t.startsWith("license:"));
    const ficha = el("li", "ficha");
    const titulo = el("h3");
    const enlace = el("a", "", r.id);
    enlace.href = "https://huggingface.co/" + r.id;
    enlace.rel = "noopener noreferrer";
    titulo.append(enlace);
    const datos = el(
      "p",
      "tenue",
      `${Number(r.downloads || 0).toLocaleString("es-ES")} descargas · licencia ${licencia ? licencia.slice(8) : "sin indicar"}`,
    );
    const comando = el("div", "comando");
    comando.append(el("code", "", `locall pull ${r.id}`), boton());
    ficha.append(titulo, datos, comando);
    lista.append(ficha);
  }
}

function el(etiqueta, clase = "", texto = "") {
  const e = document.createElement(etiqueta);
  if (clase) e.className = clase;
  if (texto) e.textContent = texto;
  return e;
}

function boton() {
  const b = el("button", "copiar", "Copiar");
  b.type = "button";
  b.setAttribute("aria-label", "Copiar el comando");
  return b;
}
