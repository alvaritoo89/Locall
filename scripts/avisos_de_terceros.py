r"""Third-party notices for what Locall distributes (ADR 0031), and the license check.

Collects every third-party component that ends up inside the installer:

- Rust crates compiled into `locall.exe` and the app (normal dependencies of the `locall` and `locall-app` packages,
  for Windows; development and build-only dependencies are not distributed and are left out);
- npm packages bundled into the app's UI (the non-development entries of `crates/locall-app/package-lock.json`);
- the engine (llama.cpp and LLVM OpenMP), whose license files travel in the `motor` folder.

For each one it writes its name, version, license and the texts of its license files, grouping components that share
exactly the same texts. With `--comprobar` it writes nothing and only checks that every license is one we accept:
it exits with an error otherwise, so a dependency with an unexpected license cannot slip into a release.

Usage, from the repository root:

    python scripts\avisos_de_terceros.py --comprobar
    python scripts\avisos_de_terceros.py --salida target\paquete\THIRD-PARTY-NOTICES.txt

It needs `cargo` and the app's `node_modules` installed (`npm ci` in crates/locall-app).
"""

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
APP = os.path.join(RAIZ, "crates", "locall-app")
RAICES_RUST = ("locall", "locall-app")
PLATAFORMA = "x86_64-pc-windows-msvc"

# Licenses we accept (SPDX identifiers): permissive ones, plus MPL-2.0 (file-level copyleft: fine to distribute
# unmodified, pointing to its source) and OFL-1.1 (fonts).
ACEPTADAS = {
    "MIT", "MIT-0", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib", "0BSD", "Unlicense", "CC0-1.0",
    "BSL-1.0", "Unicode-3.0", "Unicode-DFS-2016", "MPL-2.0", "OFL-1.1", "Apache-2.0 WITH LLVM-exception",
}
# Licenses with conditions worth stating in the notices.
CON_FUENTE = {"MPL-2.0"}

PREFIJOS_LICENCIA = ("LICEN", "COPYING", "NOTICE", "UNLICENSE", "COPYRIGHT")

MOTOR = [
    ("llama.cpp", "b12189", "MIT", r"motor\LICENSE-llama.cpp", "https://github.com/ggml-org/llama.cpp"),
    ("LLVM OpenMP runtime (libomp.dll)", "bundled with llama.cpp b12189", "Apache-2.0 WITH LLVM-exception",
     r"motor\LICENSE-LLVM-OpenMP", "https://github.com/llvm/llvm-project"),
]


# ─── License expressions (SPDX: AND, OR, WITH, parentheses; old crates use "/" for OR) ───

def fichas(expresion):
    expresion = expresion.replace("/", " OR ")
    return re.findall(r"\(|\)|[^\s()]+", expresion)


def aceptada(expresion):
    """Is the expression satisfiable with accepted licenses? OR: any branch; AND: every part."""
    t = fichas(expresion)
    pos = 0

    def termino():
        nonlocal pos
        if t[pos] == "(":
            pos += 1
            valor = o()
            pos += 1  # ")"
            return valor
        nombre = t[pos]
        pos += 1
        if pos < len(t) and t[pos] == "WITH":
            nombre = f"{nombre} WITH {t[pos + 1]}"
            pos += 2
        return nombre in ACEPTADAS

    def y():
        nonlocal pos
        valor = termino()
        while pos < len(t) and t[pos] == "AND":
            pos += 1
            valor = termino() and valor
        return valor

    def o():
        nonlocal pos
        valor = y()
        while pos < len(t) and t[pos] == "OR":
            pos += 1
            valor = y() or valor
        return valor

    try:
        resultado = o()
        return resultado and pos == len(t)
    except IndexError:
        return False


def menciona(expresion, licencia):
    return licencia in fichas(expresion)


# ─── Collecting components ───

def textos_de(carpeta, extra=None):
    """The license files of a package: (name, text) sorted by name."""
    nombres = set()
    if os.path.isdir(carpeta):
        for nombre in os.listdir(carpeta):
            if nombre.upper().startswith(PREFIJOS_LICENCIA) and os.path.isfile(os.path.join(carpeta, nombre)):
                nombres.add(nombre)
    if extra and os.path.isfile(os.path.join(carpeta, extra)):
        nombres.add(extra)
    textos = []
    for nombre in sorted(nombres):
        with open(os.path.join(carpeta, nombre), encoding="utf-8", errors="replace") as f:
            textos.append((nombre, f.read().strip().replace("\r\n", "\n")))
    return textos


def crates_distribuidos():
    salida = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", PLATAFORMA],
        cwd=RAIZ, capture_output=True, check=True,
    ).stdout
    meta = json.loads(salida)
    paquetes = {p["id"]: p for p in meta["packages"]}
    nodos = {n["id"]: n for n in meta["resolve"]["nodes"]}
    pendientes = [p["id"] for p in meta["packages"] if p["name"] in RAICES_RUST and p["source"] is None]
    if len(pendientes) != len(RAICES_RUST):
        sys.exit(f"no se encuentran los paquetes {RAICES_RUST} en el workspace")
    vistos = set()
    while pendientes:
        ident = pendientes.pop()
        if ident in vistos:
            continue
        vistos.add(ident)
        for dep in nodos[ident]["deps"]:
            # Only normal dependencies end up in the program (kind None); dev and build dependencies do not.
            if any(k["kind"] is None for k in dep["dep_kinds"]):
                pendientes.append(dep["pkg"])
    componentes = []
    for ident in vistos:
        p = paquetes[ident]
        if p["source"] is None:
            continue  # Locall's own crates
        carpeta = os.path.dirname(p["manifest_path"])
        componentes.append({
            "nombre": p["name"],
            "version": p["version"],
            "licencia": p["license"] or "(sin declarar)",
            "origen": p.get("repository") or f"https://crates.io/crates/{p['name']}",
            "fuente": f"https://crates.io/crates/{p['name']}/{p['version']}",
            "textos": textos_de(carpeta, p.get("license_file")),
            "tipo": "Rust crate",
        })
    return componentes


def paquetes_npm():
    with open(os.path.join(APP, "package-lock.json"), encoding="utf-8") as f:
        bloqueo = json.load(f)
    componentes = []
    for ruta, p in bloqueo["packages"].items():
        if not ruta or p.get("dev"):
            continue
        nombre = ruta.split("node_modules/")[-1]
        carpeta = os.path.join(APP, ruta.replace("/", os.sep))
        if not os.path.isdir(carpeta):
            if p.get("optional"):
                continue
            sys.exit(f"falta {nombre} en node_modules: ejecuta «npm ci» en crates/locall-app")
        componentes.append({
            "nombre": nombre,
            "version": p.get("version", "?"),
            "licencia": p.get("license") or "(sin declarar)",
            "origen": f"https://www.npmjs.com/package/{nombre}",
            "fuente": f"https://www.npmjs.com/package/{nombre}/v/{p.get('version', '')}",
            "textos": textos_de(carpeta),
            "tipo": "npm package",
        })
    return componentes


# ─── Writing the notices ───

def escribir(componentes, ruta):
    lineas = [
        "THIRD-PARTY NOTICES",
        "",
        "Locall is distributed under the Apache License 2.0 (see LICENSE). It includes the third-party components",
        "listed below, each under its own license. Components that share exactly the same license texts are grouped.",
        "",
        "=" * 100,
        "Inference engine (in the «motor» folder)",
        "=" * 100,
        "",
    ]
    for nombre, version, licencia, archivo, origen in MOTOR:
        lineas += [f"{nombre} {version}", f"  License: {licencia} (full text in {archivo})", f"  Source: {origen}", ""]

    con_fuente = sorted(
        (c for c in componentes if any(menciona(c["licencia"], l) for l in CON_FUENTE)),
        key=lambda c: c["nombre"],
    )
    if con_fuente:
        lineas += [
            "Components under the Mozilla Public License 2.0 are distributed unmodified. Their source code is",
            "available at:",
        ]
        lineas += [f"  {c['nombre']} {c['version']}: {c['fuente']}" for c in con_fuente]
        lineas.append("")

    grupos = {}
    for c in componentes:
        huella = hashlib.sha256(json.dumps(c["textos"]).encode("utf-8")).hexdigest()
        grupos.setdefault(huella, []).append(c)
    for miembros in sorted(grupos.values(), key=lambda g: min(m["nombre"].lower() for m in g)):
        miembros.sort(key=lambda m: (m["nombre"].lower(), m["version"]))
        lineas += ["=" * 100]
        for m in miembros:
            lineas.append(f"{m['nombre']} {m['version']} ({m['tipo']}) · License: {m['licencia']} · {m['origen']}")
        lineas += ["=" * 100, ""]
        textos = miembros[0]["textos"]
        if not textos:
            lineas += [
                "No license file is included in the published package. The license above is the one its authors",
                "declared; see the source link for its terms.",
                "",
            ]
        for archivo, texto in textos:
            lineas += [f"--- {archivo} ---", "", texto, ""]

    os.makedirs(os.path.dirname(os.path.abspath(ruta)), exist_ok=True)
    with open(ruta, "w", encoding="utf-8", newline="\r\n") as f:
        f.write("\n".join(lineas).rstrip() + "\n")


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    argumentos = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    argumentos.add_argument("--comprobar", action="store_true", help="solo comprobar las licencias")
    argumentos.add_argument("--salida", help="dónde escribir THIRD-PARTY-NOTICES.txt")
    a = argumentos.parse_args()
    if not a.comprobar and not a.salida:
        argumentos.error("indica --comprobar o --salida")

    componentes = crates_distribuidos() + paquetes_npm()
    rechazados = [c for c in componentes if not aceptada(c["licencia"])]
    for c in sorted(rechazados, key=lambda c: c["nombre"]):
        print(f"LICENCIA NO ACEPTADA: {c['nombre']} {c['version']} ({c['tipo']}): {c['licencia']}")
    if rechazados:
        sys.exit(f"{len(rechazados)} componentes con una licencia no aceptada (ADR 0031)")

    rust = sum(1 for c in componentes if c["tipo"] == "Rust crate")
    print(f"Licencias correctas: {rust} crates de Rust, {len(componentes) - rust} paquetes npm y el motor.")
    if a.salida:
        escribir(componentes, a.salida)
        print(f"Avisos escritos en {a.salida} ({os.path.getsize(a.salida) / 1024:.0f} KB)")


if __name__ == "__main__":
    main()
