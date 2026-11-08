r"""Prueba de Locall con el cliente OFICIAL de OpenAI (paquete `openai` de Python). ADR 0016.

No forma parte de `cargo test` porque necesita instalar el paquete `openai`. Para repetirla:

    python -m venv %TEMP%\venv-openai
    %TEMP%\venv-openai\Scripts\python -m pip install openai
    (con `locall serve` en marcha y un modelo instalado como «qwen»)
    %TEMP%\venv-openai\Scripts\python scripts\probar_openai.py %LOCALAPPDATA%\Locall\token 11470

Comprueba: lista de modelos, respuesta entera, por trozos con el uso al final, y que los errores (función no
admitida, clave incorrecta, modelo inexistente) llegan como las excepciones que espera el cliente.
"""

import sys

import openai
from openai import OpenAI

token = open(sys.argv[1], encoding="utf-8").read().strip()
base = f"http://127.0.0.1:{sys.argv[2]}/v1"
cliente = OpenAI(base_url=base, api_key=token, timeout=120)
fallos = 0


def comprobar(nombre, condicion, detalle=""):
    global fallos
    print(("OK   " if condicion else "FALLO") + f" {nombre}" + (f"  ({detalle})" if detalle else ""))
    if not condicion:
        fallos += 1


print(f"openai {openai.__version__} contra {base}\n")

# 1) Lista de modelos
modelos = cliente.models.list()
ids = [m.id for m in modelos.data]
comprobar("models.list()", "qwen" in ids, f"modelos: {ids}")

# 2) Respuesta entera
r = cliente.chat.completions.create(
    model="qwen",
    messages=[
        {"role": "system", "content": "Responde en una palabra."},
        {"role": "user", "content": "¿Capital de Francia?"},
    ],
    max_tokens=10,
    temperature=0,
    seed=1,
)
texto = r.choices[0].message.content
comprobar("chat.completions.create()", "Par" in texto, f"respuesta: {texto!r}")
comprobar("  objeto y rol", r.object == "chat.completion" and r.choices[0].message.role == "assistant")
comprobar(
    "  uso de tokens",
    r.usage.total_tokens == r.usage.prompt_tokens + r.usage.completion_tokens > 0,
    f"{r.usage.prompt_tokens} + {r.usage.completion_tokens}",
)
comprobar("  motivo del final", r.choices[0].finish_reason in ("stop", "length"), r.choices[0].finish_reason)

# 3) Por trozos (SSE), con el uso al final
trozos = list(
    cliente.chat.completions.create(
        model="qwen",
        messages=[{"role": "user", "content": "¿Capital de Italia? Una palabra."}],
        max_tokens=10,
        temperature=0,
        seed=1,
        stream=True,
        stream_options={"include_usage": True},
    )
)
texto = "".join(t.choices[0].delta.content or "" for t in trozos if t.choices)
comprobar("stream=True", "Roma" in texto, f"{len(trozos)} trozos, respuesta: {texto!r}")
comprobar("  uso en el último trozo", trozos[-1].usage is not None and trozos[-1].usage.completion_tokens > 0)

# 4) Lo no admitido: error claro (400), no silencio
try:
    cliente.chat.completions.create(
        model="qwen",
        messages=[{"role": "user", "content": "hola"}],
        tools=[{"type": "function", "function": {"name": "f", "parameters": {"type": "object"}}}],
    )
    comprobar("tools rechazado", False, "no dio error")
except openai.BadRequestError as e:
    comprobar("tools rechazado con BadRequestError (400)", "tools" in str(e), str(e)[:90])

# 5) Clave incorrecta
try:
    OpenAI(base_url=base, api_key="0" * 64).models.list()
    comprobar("clave incorrecta rechazada", False, "no dio error")
except openai.AuthenticationError as e:
    comprobar("clave incorrecta con AuthenticationError (401)", True, str(e)[:90])

# 6) Modelo que no existe
try:
    cliente.chat.completions.create(model="no-existe", messages=[{"role": "user", "content": "hola"}])
    comprobar("modelo inexistente", False, "no dio error")
except openai.NotFoundError as e:
    comprobar("modelo inexistente con NotFoundError (404)", True, str(e)[:90])

print(f"\n{'TODO BIEN' if fallos == 0 else f'{fallos} FALLOS'}")
sys.exit(1 if fallos else 0)
