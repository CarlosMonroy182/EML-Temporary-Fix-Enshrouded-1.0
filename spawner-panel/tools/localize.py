import os
import re
import shutil
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from i18n_en import EN

SPANISH_MARKS = re.compile(r"[áéíóúñÁÉÍÓÚÑ¿¡«»]")
SPANISH_WORDS = {
    "el", "la", "los", "las", "del", "al", "una", "para", "con", "sin", "tu", "tus", "por",
    "que", "como", "mensaje", "mensajes", "jugador", "jugadores", "equipo", "zona", "nombre",
    "guardado", "enviado", "borrar", "elige", "abre", "juego", "servidor", "vida", "radio",
    "nadie", "esperando", "criaturas", "lugar", "lugares", "cerrar",
}
PROTECTED = {"vivo", "creando", "esperando para reaparecer", "esperando el mundo", "sin foco", "/m182 k ñ", "Zona número {i} ñandú"}


def lex(src):
    out = []
    i = 0
    n = len(src)
    while i < n:
        c = src[i]
        if src.startswith("//", i):
            j = src.find("\n", i)
            j = n if j < 0 else j
            out.append(("code", src[i:j]))
            i = j
        elif src.startswith("/*", i):
            depth = 0
            j = i
            while j < n:
                if src.startswith("/*", j):
                    depth += 1
                    j += 2
                elif src.startswith("*/", j):
                    depth -= 1
                    j += 2
                    if depth == 0:
                        break
                else:
                    j += 1
            out.append(("code", src[i:j]))
            i = j
        elif c in "rb" and (i == 0 or not (src[i - 1].isalnum() or src[i - 1] == "_")) and re.match(r'(br|r)#*"', src[i:i + 12]):
            m = re.match(r'(br|r)(#*)"', src[i:])
            close = '"' + m.group(2)
            j = src.find(close, i + m.end())
            j = n if j < 0 else j + len(close)
            out.append(("code", src[i:j]))
            i = j
        elif c == "'":
            m = re.match(r"'(\\.[^']*|[^\\'])'", src[i:])
            if m:
                out.append(("code", m.group(0)))
                i += m.end()
            else:
                out.append(("code", c))
                i += 1
        elif c == '"':
            j = i + 1
            while j < n and src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            out.append(("str", src[i + 1:j]))
            i = j + 1
        else:
            j = i + 1
            while j < n and src[j] not in "/\"'rb":
                j += 1
            out.append(("code", src[i:j]))
            i = j
    return out


def looks_spanish(text):
    if text in PROTECTED:
        return False
    if SPANISH_MARKS.search(text):
        return True
    words = re.findall(r"[A-Za-z]+", text.lower())
    return any(w in SPANISH_WORDS for w in words)


def main():
    src_dir, out_dir = sys.argv[1], sys.argv[2]
    if os.path.exists(out_dir):
        shutil.rmtree(out_dir)
    shutil.copytree(src_dir, out_dir)
    used = set()
    leftovers = []
    for name in sorted(os.listdir(out_dir)):
        if not name.endswith(".rs"):
            continue
        path = os.path.join(out_dir, name)
        text = open(path, encoding="utf-8").read()
        parts = []
        for kind, chunk in lex(text):
            if kind == "str":
                if chunk in EN:
                    used.add(chunk)
                    chunk = EN[chunk]
                elif looks_spanish(chunk):
                    leftovers.append(f"{name}: {chunk[:100]}")
                parts.append('"' + chunk + '"')
            else:
                parts.append(chunk)
        open(path, "w", encoding="utf-8").write("".join(parts))
    unused = [k for k in EN if k not in used]
    for k in unused:
        print("unused key:", k)
    for l in leftovers:
        print("untranslated:", l)
    if unused or leftovers:
        sys.exit(1)
    print(f"localized {len(used)} texts")


main()
