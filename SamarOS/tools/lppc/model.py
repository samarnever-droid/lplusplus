from __future__ import annotations

from dataclasses import dataclass, field

KEYWORDS = {
    "def", "struct", "const", "extern", "if", "elif", "else", "while", "for",
    "in", "range", "return", "break", "continue", "pass", "mut", "and", "or",
    "not", "true", "false", "null", "import", "from",
}

PRIMITIVES = {
    "Int": "int",
    "Bool": "int",
    "Str": "const char *",
    "Void": "void",
    "List": "void *",
    "Ptr": "void *",
}


class LppError(Exception):
    def __init__(self, msg, line=0, file=""):
        super().__init__(f"{file}:{line}: {msg}")


# --------------------------------------------------------------------------
# lexer
# --------------------------------------------------------------------------
@dataclass
class Tok:
    kind: str          # name num str op newline indent dedent eof
    value: str
    line: int


OPS = [
    "->", ":=", "==", "!=", "<=", ">=", "<<", ">>", "+=", "-=", "*=", "/=",
    "(", ")", "[", "]", "{", "}", ",", ":", ".", "+", "-", "*", "/", "%",
    "<", ">", "=", "&", "|", "^", "~",
]


# AST
# --------------------------------------------------------------------------
@dataclass
class Param:
    name: str
    type: str


@dataclass
class Func:
    name: str
    params: list[Param]
    ret: str
    body: list
    line: int
    extern: bool = False


@dataclass
class Struct:
    name: str
    fields: list[Param]
    line: int


@dataclass
class Const:
    name: str
    expr: object
    line: int


@dataclass
class Node:
    kind: str
    line: int = 0
    a: object = None
    b: object = None
    c: object = None
    items: list = field(default_factory=list)
    text: str = ""


# --------------------------------------------------------------------------
# parser
# --------------------------------------------------------------------------

BUILTINS = {
    # name:        (C name,        return type, arg types)
    "len":            ("str_len", "Int", ["Str"]),
    "str_len":        ("str_len", "Int", ["Str"]),
    "str_concat":     ("str_concat", "Str", ["Str", "Str"]),
    "str_eq":         ("str_eq", "Bool", ["Str", "Str"]),
    "str_starts_with": ("str_starts_with", "Bool", ["Str", "Str"]),
    "str_index_of":   ("str_index_of", "Int", ["Str", "Int"]),
    "char_at":        ("char_at", "Int", ["Str", "Int"]),
    "substr":         ("substr", "Str", ["Str", "Int", "Int"]),
    "chr":            ("chr", "Str", ["Int"]),
    "int_to_str":     ("int_to_str", "Str", ["Int"]),
    "pad2":           ("pad2", "Str", ["Int"]),
    "list_new":       ("list_new", "List", []),
    "list_push":      ("list_push", "Void", ["List", "Int"]),
    "list_get":       ("list_get", "Int", ["List", "Int"]),
    "list_set":       ("list_set", "Void", ["List", "Int", "Int"]),
    "list_len":       ("list_len", "Int", ["List"]),
    "list_remove":    ("list_remove", "Void", ["List", "Int"]),
    "list_insert":    ("list_insert", "Void", ["List", "Int", "Int"]),
    "list_clear":     ("list_clear", "Void", ["List"]),
    "abs":            ("lpp_abs", "Int", ["Int"]),
    "min":            ("lpp_min", "Int", ["Int", "Int"]),
    "max":            ("lpp_max", "Int", ["Int", "Int"]),
    "clamp":          ("lpp_clamp", "Int", ["Int", "Int", "Int"]),
    "isqrt":          ("isqrt", "Int", ["Int"]),
    "sin_deg":        ("sin_deg", "Int", ["Int"]),
    "cos_deg":        ("cos_deg", "Int", ["Int"]),
    "alloc":          ("lpp_alloc", "Ptr", ["Int"]),
}


# --------------------------------------------------------------------------
# code generator
# --------------------------------------------------------------------------
C_RESERVED = {
    "auto", "break", "case", "char", "const", "continue", "default", "do",
    "double", "else", "enum", "extern", "float", "for", "goto", "if", "inline",
    "int", "long", "register", "restrict", "return", "short", "signed",
    "sizeof", "static", "struct", "switch", "typedef", "union", "unsigned",
    "void", "volatile", "while", "bool", "true", "false", "asm", "typeof",
    "self", "main", "NULL",
}


def cid(name: str) -> str:
    """Escape L++ identifiers that collide with C keywords."""
    return name + "_" if name in C_RESERVED else name


def cname(name: str) -> str:
    """L++ `main` becomes the kernel entry point `samar_main`."""
    return "samar_main" if name == "main" else f"lx_{name}"


def escape_c(s: str) -> str:
    out = []
    for ch in s:
        if ch == "\\":
            out.append("\\\\")
        elif ch == '"':
            out.append('\\"')
        elif ch == "\n":
            out.append("\\n")
        elif ch == "\t":
            out.append("\\t")
        elif ord(ch) < 32 or ord(ch) > 126:
            out.append("\\%03o" % ord(ch))
        else:
            out.append(ch)
    return "".join(out)


