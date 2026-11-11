//! # locall-web
//!
//! Generates the Locall website (ADR 0024): static HTML pages from the **signed catalog**, so the website and the app
//! always say the same thing. No new dependencies: the HTML is written by hand and **all catalog text is escaped**
//! ([`escapar`]), even though it is signed.
//!
//! Pages: home, download («Próximamente» until Phase 8), the model library (with filters), one page per model,
//! privacy and the 404 error page. Links are relative, so the site works the same at `alvaritoo89.github.io/Locall/`
//! as on a custom domain or locally.

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

use locall_catalogo::{Catalogo, ModeloCatalogo};

/// Where the code lives (header and footer links).
pub const REPOSITORIO: &str = "https://github.com/alvaritoo89/Locall";

/// The site's path on GitHub Pages. Only used by the 404 page, which GitHub serves at any path (relative links would
/// not work there).
pub const BASE_404: &str = "/Locall/";

/// Escapes text for HTML (content, or an attribute in double quotes).
pub fn escapar(texto: &str) -> String {
    let mut s = String::with_capacity(texto.len());
    for c in texto.chars() {
        match c {
            '&' => s.push_str("&amp;"),
            '<' => s.push_str("&lt;"),
            '>' => s.push_str("&gt;"),
            '"' => s.push_str("&quot;"),
            '\'' => s.push_str("&#39;"),
            _ => s.push(c),
        }
    }
    s
}

/// `2497280256` → `2,5 GB` (decimal units, like the app and Hugging Face; Spanish decimal comma).
pub fn tamano(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb >= 1.0 {
        format!("{gb:.1} GB").replace('.', ",")
    } else {
        format!("{:.0} MB", bytes as f64 / 1e6)
    }
}

/// `40960` → `40.960` (Spanish thousands separator)
fn miles(n: u64) -> String {
    let s = n.to_string();
    let mut r = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            r.push('.');
        }
        r.push(c);
    }
    r
}

// ─── Common template ──────────────────────────────────────────────────────────

/// Which menu entry is highlighted.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Seccion {
    Inicio,
    Modelos,
    Descargar,
    Otra,
}

/// A complete page. `raiz` is the path relative to the site root (`""` or `"../"`); the content is already escaped.
fn pagina(
    raiz: &str,
    titulo: &str,
    descripcion: &str,
    seccion: Seccion,
    contenido: &str,
) -> String {
    let actual = |s: Seccion| {
        if s == seccion {
            r##" aria-current="page""##
        } else {
            ""
        }
    };
    format!(
        r##"<!doctype html>
<html lang="es">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light dark">
<title>{titulo}</title>
<meta name="description" content="{descripcion}">
<link rel="icon" href="{raiz}icono.png">
<link rel="stylesheet" href="{raiz}estilo.css">
<script src="{raiz}web.js" defer></script>
</head>
<body>
<a class="saltar" href="#principal">Saltar al contenido</a>
<header class="cabecera">
<div class="cabecera-dentro">
<a class="logo" href="{raiz}index.html" aria-label="Locall, inicio"{inicio}><picture><source srcset="{raiz}logo-oscuro.png" media="(prefers-color-scheme: dark)"><img src="{raiz}logo-claro.png" alt="Locall." width="96" height="28"></picture></a>
<nav class="navegacion" aria-label="Principal">
<a href="{raiz}modelos/index.html"{modelos}>Modelos</a>
<a href="{raiz}descargar.html"{descargar}>Descargar</a>
<a href="{REPOSITORIO}">GitHub</a>
</nav>
</div>
</header>
<main id="principal" class="contenido">
{contenido}
</main>
<footer class="pie">
<div class="pie-dentro">
<span>Locall · software libre (Apache-2.0)</span>
<a href="{REPOSITORIO}">Código en GitHub</a>
<a href="{raiz}privacidad.html">Privacidad</a>
</div>
</footer>
</body>
</html>
"##,
        titulo = escapar(titulo),
        descripcion = escapar(descripcion),
        inicio = actual(Seccion::Inicio),
        modelos = actual(Seccion::Modelos),
        descargar = actual(Seccion::Descargar),
    )
}

/// A command with its «Copiar» (copy) button (the button only appears with JavaScript).
fn comando(texto: &str) -> String {
    format!(
        r##"<div class="comando"><code>{}</code><button type="button" class="copiar" aria-label="Copiar el comando">Copiar</button></div>"##,
        escapar(texto)
    )
}

const AVISO_COMPATIBLE: &str = "Todos los modelos del catálogo son <strong>compatibles</strong>: el motor de Locall \
reconoce su formato. No se han probado uno a uno. Al instalarlo, la app te dice si cabe en tu equipo.";

// ─── Pages ────────────────────────────────────────────────────────────────────

fn inicio(catalogo: &Catalogo) -> String {
    let contenido = format!(
        r##"<section class="portada">
<h1>Modelos de IA en tu propio ordenador.</h1>
<p class="entradilla">Locall instala y ejecuta modelos de inteligencia artificial en Windows, sin cuentas, sin
suscripciones y sin enviar tus conversaciones a ningún sitio.</p>
<div class="botones">
<a class="boton primario" href="modelos/index.html">Ver los {n} modelos</a>
<a class="boton" href="descargar.html">Descargar</a>
</div>
</section>
<ul class="puntos">
<li><h3>Privado</h3><p>Todo ocurre en tu equipo. Sin estadísticas ni telemetría, y lo que borras se borra de verdad.</p></li>
<li><h3>Para cualquier equipo</h3><p>Pensado también para portátiles sin gráfica dedicada. Te dice si un modelo cabe antes de descargarlo.</p></li>
<li><h3>Verificado</h3><p>Cada descarga va fijada a una versión exacta y se comprueba con sha256. El catálogo va firmado.</p></li>
<li><h3>Libre</h3><p>Código abierto con licencia Apache-2.0. Gratis, sin servicios de pago obligatorios.</p></li>
</ul>
<h2>Así de sencillo</h2>
<p class="tenue">Desde la app, con un botón. O desde la línea de comandos:</p>
{pull}
<p></p>
{run}
<h2>¿En qué punto está?</h2>
<p class="aviso">Locall está en desarrollo y todavía no hay una versión para descargar: llegará con un instalador
firmado. El código ya es público en <a href="{REPOSITORIO}">GitHub</a>.</p>
"##,
        n = catalogo.modelos.len(),
        pull = comando("locall pull qwen3-4b"),
        run = comando("locall run qwen3-4b"),
    );
    pagina(
        "",
        "Locall · modelos de IA en tu ordenador",
        "Instala y ejecuta modelos de IA en Windows, en tu propio equipo: privado, gratis y de código abierto.",
        Seccion::Inicio,
        &contenido,
    )
}

fn descargar() -> String {
    let contenido = format!(
        r##"<h1>Descargar</h1>
<p class="entradilla">Próximamente.</p>
<div class="aviso">
<p>Locall está en desarrollo. La versión para Windows llegará con un <strong>instalador firmado</strong> y un comando
de instalación de una línea, para que Windows no la marque como sospechosa.</p>
<p>Mientras tanto, el código es público y se puede compilar siguiendo el
<a href="{REPOSITORIO}#probarlo">README</a>.</p>
</div>
<h2>Lo que necesitarás</h2>
<ul>
<li>Windows 11 de 64 bits (en Windows 10 todavía no se ha comprobado).</li>
<li>Orientativo: 8 GB de RAM para los modelos pequeños y 16 GB para los medianos. La app te dice si cada modelo cabe
en tu equipo antes de descargarlo.</li>
<li>No hace falta tarjeta gráfica.</li>
</ul>
"##
    );
    pagina(
        "",
        "Descargar · Locall",
        "Descargar Locall para Windows: próximamente.",
        Seccion::Descargar,
        &contenido,
    )
}

/// The uses that have a filter, with their display name.
const USOS: [(&str, &str); 2] = [("conversar", "Conversar"), ("razonar", "Razonar")];

fn biblioteca(catalogo: &Catalogo) -> String {
    let mut fichas = String::new();
    for m in &catalogo.modelos {
        let v = m.por_defecto();
        let _ = write!(
            fichas,
            r##"<li class="ficha" data-etiquetas="{etiquetas}" data-gb="{gb:.1}" data-texto="{texto}">
<h3><a href="{nombre}.html">{titulo}</a></h3>
<p class="tenue">{autor} · {tamano} · licencia {licencia}</p>
<p>{descripcion}</p>
<p><span class="etiqueta">compatible</span></p>
{comando}
</li>
"##,
            etiquetas = escapar(&m.etiquetas.join(" ")),
            texto = escapar(
                &format!("{} {} {} {}", m.nombre, m.titulo, m.autor, m.descripcion).to_lowercase()
            ),
            gb = v.bytes() as f64 / 1e9,
            nombre = escapar(&m.nombre),
            titulo = escapar(&m.titulo),
            autor = escapar(&m.autor),
            tamano = tamano(v.bytes()),
            licencia = escapar(&m.licencia),
            descripcion = escapar(&m.descripcion),
            comando = comando(&format!("locall pull {}", m.nombre)),
        );
    }
    let mut usos = String::new();
    for (valor, nombre) in USOS {
        let _ = write!(
            usos,
            r##"<button type="button" data-filtro="etiqueta" data-valor="{valor}" aria-pressed="false">{nombre}</button>"##
        );
    }
    let contenido = format!(
        r##"<h1>Modelos</h1>
<p class="entradilla">Los modelos que recomienda Locall. Cada uno, fijado a una versión exacta y comprobado con
sha256 al descargarlo.</p>
<form class="buscador" role="search" action="#">
<label for="buscar" class="solo-lectores">Buscar modelos</label>
<input id="buscar" type="search" placeholder="Busca un modelo: qwen, llama, gemma…" maxlength="100" autocomplete="off" spellcheck="false">
<button type="submit" class="boton">Buscar en Hugging Face</button>
</form>
<p class="tenue pequeno buscador-nota">Al escribir se filtra este catálogo, sin enviar nada. El botón busca entre todos
los modelos GGUF de Hugging Face: solo entonces tu navegador le envía a Hugging Face lo que has escrito.</p>
<p class="tenue pequeno">{AVISO_COMPATIBLE}</p>
<div class="filtros" role="group" aria-label="Filtrar">
<button type="button" data-filtro="etiqueta" data-valor="todos" aria-pressed="true">Todos</button>{usos}
<span aria-hidden="true">·</span>
<button type="button" data-filtro="tamano" data-valor="todos" aria-pressed="true">Cualquier tamaño</button><button type="button" data-filtro="tamano" data-valor="pequeno" aria-pressed="false">Menos de 2 GB</button><button type="button" data-filtro="tamano" data-valor="mediano" aria-pressed="false">De 2 a 6 GB</button><button type="button" data-filtro="tamano" data-valor="grande" aria-pressed="false">Más de 6 GB</button>
</div>
<p class="tenue pequeno cuenta" aria-live="polite">{n} modelos</p>
<ul class="modelos">
{fichas}</ul>
<section class="resultados-hf" hidden aria-labelledby="titulo-hf">
<h2 id="titulo-hf">En Hugging Face</h2>
<p class="tenue pequeno">Estos no están en el catálogo de Locall. Al instalarlos, Locall fija la versión exacta, comprueba
si su motor puede usarlos antes de descargarlos y verifica cada archivo con sha256. Algunos exigen aceptar condiciones
con una cuenta de Hugging Face: esos, Locall todavía no puede descargarlos.</p>
<p class="estado-hf" role="status"></p>
<ul class="modelos lista-hf"></ul>
</section>
"##,
        n = catalogo.modelos.len(),
    );
    pagina(
        "../",
        "Modelos · Locall",
        "Los modelos de IA que recomienda Locall, con su comando para instalarlos.",
        Seccion::Modelos,
        &contenido,
    )
}

fn ficha_de_modelo(m: &ModeloCatalogo) -> String {
    let v = m.por_defecto();
    let mut archivos = String::new();
    for a in &v.archivos {
        let _ = write!(
            archivos,
            r##"<dt>Archivo</dt><dd class="mono">{ruta} ({tamano})</dd><dt>sha256</dt><dd class="mono pequeno">{sha}</dd>"##,
            ruta = escapar(&a.ruta),
            tamano = tamano(a.bytes),
            sha = escapar(&a.sha256),
        );
    }
    let contenido = format!(
        r##"<p class="pequeno"><a href="index.html">← Todos los modelos</a></p>
<h1>{titulo}</h1>
<p class="entradilla">{descripcion}</p>
<p><span class="etiqueta">compatible</span></p>
<h2>Instalar</h2>
<p class="tenue">Desde la app: Explorar → Recomendados por Locall. O desde la línea de comandos:</p>
{pull}
<p></p>
{run}
<dl class="datos">
<dt>Autor</dt><dd>{autor}</dd>
<dt>Licencia</dt><dd>{licencia}</dd>
<dt>Tamaño de descarga</dt><dd>{tamano}</dd>
<dt>Contexto máximo</dt><dd>{contexto} tokens</dd>
<dt>Versión</dt><dd>{variante}</dd>
<dt>Repositorio</dt><dd><a href="https://huggingface.co/{repo_url}/tree/{commit}">{repo}</a></dd>
<dt>Commit</dt><dd class="mono pequeno">{commit}</dd>
{archivos}
<dt>Compatibilidad</dt><dd>El motor de Locall ({motor}) reconoce su formato. No se ha probado uno a uno.</dd>
</dl>
<p class="tenue pequeno">La licencia del modelo es la de su autor. Antes de usarlo, revísala en su repositorio.</p>
"##,
        titulo = escapar(&m.titulo),
        descripcion = escapar(&m.descripcion),
        pull = comando(&format!("locall pull {}", m.nombre)),
        run = comando(&format!("locall run {}", m.nombre)),
        autor = escapar(&m.autor),
        licencia = escapar(&m.licencia),
        tamano = tamano(v.bytes()),
        contexto = miles(m.contexto_max),
        variante = escapar(&v.nombre.to_uppercase()),
        repo = escapar(&v.repo),
        repo_url = escapar(&v.repo),
        commit = escapar(&v.commit),
        motor = escapar(&v.motor_comprobado),
    );
    pagina(
        "../",
        &format!("{} · Locall", m.titulo),
        &format!("{}: {}", m.titulo, m.descripcion),
        Seccion::Modelos,
        &contenido,
    )
}

fn privacidad() -> String {
    let contenido = r##"<h1>Privacidad</h1>
<p class="entradilla">Locall está hecho para que tus datos se queden en tu equipo.</p>
<h2>Esta web</h2>
<ul>
<li>No usa cookies, ni estadísticas, ni nada de otros sitios: las fuentes y todo lo demás van en la propia web.</li>
<li>El buscador de la página de modelos filtra el catálogo sin enviar nada. Solo si pulsas «Buscar en Hugging Face»,
tu navegador envía a Hugging Face lo que has escrito, para buscar entre todos sus modelos.</li>
<li>Está alojada en GitHub Pages. Según GitHub, al visitar una web de GitHub Pages <strong>se registra la dirección IP
de quien la visita</strong>, por seguridad. Locall no tiene acceso a esos datos.
Más información en la <a href="https://docs.github.com/es/site-policy/privacy-policies/github-general-privacy-statement">declaración de privacidad de GitHub</a>.</li>
</ul>
<h2>La app</h2>
<ul>
<li>Tus conversaciones, ajustes y modelos se guardan solo en tu equipo. Al borrar una conversación, se borra de
verdad, también del disco.</li>
<li>Locall solo se conecta a internet para buscar y descargar modelos de Hugging Face y, como mucho una vez al día,
para mirar en GitHub si hay un catálogo de modelos nuevo.</li>
<li>No envía estadísticas, ni datos de uso, ni tus conversaciones a ningún sitio. No hay nada que desactivar.</li>
</ul>
"##;
    pagina(
        "",
        "Privacidad · Locall",
        "Qué datos usa Locall y su web: ninguno tuyo sale de tu equipo.",
        Seccion::Otra,
        contenido,
    )
}

fn no_encontrada() -> String {
    // GitHub serves this page at any path that does not exist: relative links would break, so the base is used.
    pagina(
        BASE_404,
        "Página no encontrada · Locall",
        "Esta página no existe.",
        Seccion::Otra,
        r##"<h1>Esta página no existe.</h1>
<p class="entradilla">Puede que se haya movido. Vuelve al <a href="/Locall/index.html">inicio</a> o mira los
<a href="/Locall/modelos/index.html">modelos</a>.</p>
"##,
    )
}

// ─── Generating ───────────────────────────────────────────────────────────────

/// The site's pages: (relative path, content).
pub fn paginas(catalogo: &Catalogo) -> Vec<(PathBuf, String)> {
    let mut lista = vec![
        (PathBuf::from("index.html"), inicio(catalogo)),
        (PathBuf::from("descargar.html"), descargar()),
        (PathBuf::from("privacidad.html"), privacidad()),
        (PathBuf::from("404.html"), no_encontrada()),
        (
            PathBuf::from("modelos").join("index.html"),
            biblioteca(catalogo),
        ),
    ];
    for m in &catalogo.modelos {
        lista.push((
            PathBuf::from("modelos").join(format!("{}.html", m.nombre)),
            ficha_de_modelo(m),
        ));
    }
    lista
}

/// Writes the site to `destino`: the pages, the files in `estaticos` (style, script, fonts, logos) and a copy of the
/// catalog with its signature in `catalogo/`. Returns the files written.
pub fn generar(
    catalogo: &Catalogo,
    datos_catalogo: &[u8],
    firma: &str,
    estaticos: &Path,
    destino: &Path,
) -> io::Result<Vec<PathBuf>> {
    let mut escritos = Vec::new();
    std::fs::create_dir_all(destino.join("modelos"))?;
    std::fs::create_dir_all(destino.join("catalogo"))?;
    for (ruta, html) in paginas(catalogo) {
        let ruta = destino.join(ruta);
        std::fs::write(&ruta, html)?;
        escritos.push(ruta);
    }
    copiar_carpeta(estaticos, destino, &mut escritos)?;
    for (nombre, contenido) in [
        ("catalogo.json", datos_catalogo),
        ("catalogo.json.minisig", firma.as_bytes()),
    ] {
        let ruta = destino.join("catalogo").join(nombre);
        std::fs::write(&ruta, contenido)?;
        escritos.push(ruta);
    }
    Ok(escritos)
}

fn copiar_carpeta(origen: &Path, destino: &Path, escritos: &mut Vec<PathBuf>) -> io::Result<()> {
    std::fs::create_dir_all(destino)?;
    for entrada in std::fs::read_dir(origen)? {
        let entrada = entrada?;
        let ruta = destino.join(entrada.file_name());
        if entrada.file_type()?.is_dir() {
            copiar_carpeta(&entrada.path(), &ruta, escritos)?;
        } else {
            std::fs::copy(entrada.path(), &ruta)?;
            escritos.push(ruta);
        }
    }
    Ok(())
}

#[cfg(test)]
mod pruebas;
