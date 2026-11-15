from __future__ import annotations

from .model import LppError, Tok, OPS

def tokenize(src: str, fname: str) -> list[Tok]:
    toks: list[Tok] = []
    indents = [0]
    depth = 0
    lines = src.split("\n")
    for lineno, raw in enumerate(lines, 1):
        line = raw.replace("\t", "    ")

        # strip comments outside string literals
        out, in_str, i = [], False, 0
        while i < len(line):
            c = line[i]
            if in_str:
                out.append(c)
                if c == "\\" and i + 1 < len(line):
                    out.append(line[i + 1])
                    i += 2
                    continue
                if c == '"':
                    in_str = False
            else:
                if c == "#":
                    break
                out.append(c)
                if c == '"':
                    in_str = True
            i += 1
        line = "".join(out)

        if not line.strip():
            continue

        if depth == 0:
            width = len(line) - len(line.lstrip(" "))
            if width > indents[-1]:
                indents.append(width)
                toks.append(Tok("indent", "", lineno))
            while width < indents[-1]:
                indents.pop()
                toks.append(Tok("dedent", "", lineno))
                if width > indents[-1]:
                    raise LppError("inconsistent indentation", lineno, fname)

        i = 0
        body = line
        while i < len(body):
            c = body[i]
            if c == " ":
                i += 1
                continue
            if c.isalpha() or c == "_":
                j = i
                while j < len(body) and (body[j].isalnum() or body[j] == "_"):
                    j += 1
                toks.append(Tok("name", body[i:j], lineno))
                i = j
                continue
            if c.isdigit():
                j = i
                if body.startswith("0x", i) or body.startswith("0X", i):
                    j = i + 2
                    while j < len(body) and (body[j] in "0123456789abcdefABCDEF_"):
                        j += 1
                else:
                    while j < len(body) and (body[j].isdigit() or body[j] == "_"):
                        j += 1
                toks.append(Tok("num", body[i:j].replace("_", ""), lineno))
                i = j
                continue
            if c == '"':
                j = i + 1
                buf = []
                while j < len(body) and body[j] != '"':
                    if body[j] == "\\" and j + 1 < len(body):
                        esc = body[j + 1]
                        buf.append({"n": "\n", "t": "\t", '"': '"', "\\": "\\",
                                    "0": "\0", "r": "\r"}.get(esc, esc))
                        j += 2
                        continue
                    buf.append(body[j])
                    j += 1
                if j >= len(body):
                    raise LppError("unterminated string", lineno, fname)
                toks.append(Tok("str", "".join(buf), lineno))
                i = j + 1
                continue
            for op in OPS:
                if body.startswith(op, i):
                    if op in "([":
                        depth += 1
                    elif op in ")]":
                        depth = max(0, depth - 1)
                    toks.append(Tok("op", op, lineno))
                    i += len(op)
                    break
            else:
                raise LppError(f"unexpected character {c!r}", lineno, fname)

        if depth == 0:
            toks.append(Tok("newline", "", lineno))

    while len(indents) > 1:
        indents.pop()
        toks.append(Tok("dedent", "", len(lines)))
    toks.append(Tok("eof", "", len(lines)))
    return toks


# --------------------------------------------------------------------------
