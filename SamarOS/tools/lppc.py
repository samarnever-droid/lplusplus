#!/usr/bin/env python3
"""lppc — the SamarOS L++ kernel compiler.

Compiles the *kernel profile* of L++ (the subset that can run with no libc,
no OS and no allocator beyond the kernel bump heap) into freestanding C99,
which gcc then turns into the 32-bit kernel image.

Why a second front end?  The main `lpp` compiler in this repository is a
Cranelift AOT pipeline that emits hosted ELF/PE/Mach-O executables linked
against lpp_runtime.c (libc, syscalls, threads).  A kernel cannot use any of
that, and building the Rust compiler is not possible in every environment,
so SamarOS ships its own small, dependency-free front end for the same
language.  The syntax accepted here is a strict subset of L++:

    extern "C":                    FFI declaration blocks
    const NAME = expr              module level constants
    struct Name:                   heap structs + generated constructors
        field: Type
    def name(a: Int) -> Int:       functions, Python-style blocks
    x := expr / mut x := expr      declarations
    if / elif / else / while       control flow
    for i in range(a, b):          counted loops
    return / break / continue      jumps
    and / or / not / true / false  logical keywords

Supported types: Int, Bool, Str, Void, List, Ptr and user structs.
Builtins mirror the L++ standard builtin set (len, str_concat, int_to_str,
char_at, substr, chr, list_new, list_push, list_get, list_set, list_len,
list_remove, list_insert, list_clear, abs, min, max, clamp, isqrt,
sin_deg, cos_deg, str_eq, str_starts_with, str_index_of, pad2).

    usage: lppc.py out.c file1.lpp file2.lpp ...
"""
from __future__ import annotations

import sys
from dataclasses import dataclass, field
from typing import Optional

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
class Parser:
    def __init__(self, toks: list[Tok], fname: str):
        self.toks = toks
        self.i = 0
        self.fname = fname

    # -- helpers
    def peek(self, k=0) -> Tok:
        return self.toks[min(self.i + k, len(self.toks) - 1)]

    def next(self) -> Tok:
        t = self.toks[self.i]
        self.i += 1
        return t

    def at(self, kind, value=None) -> bool:
        t = self.peek()
        return t.kind == kind and (value is None or t.value == value)

    def accept(self, kind, value=None) -> Optional[Tok]:
        if self.at(kind, value):
            return self.next()
        return None

    def expect(self, kind, value=None) -> Tok:
        t = self.peek()
        if not self.at(kind, value):
            raise LppError(
                f"expected {value or kind}, found {t.value or t.kind!r}", t.line, self.fname
            )
        return self.next()

    def err(self, msg):
        raise LppError(msg, self.peek().line, self.fname)

    # -- module
    def parse_module(self):
        funcs, structs, consts = [], [], []
        while not self.at("eof"):
            if self.accept("newline"):
                continue
            t = self.peek()
            if t.kind == "name" and t.value in ("import", "from"):
                while not self.at("newline") and not self.at("eof"):
                    self.next()
                continue
            if t.kind == "name" and t.value == "extern":
                funcs.extend(self.parse_extern())
            elif t.kind == "name" and t.value == "struct":
                structs.append(self.parse_struct())
            elif t.kind == "name" and t.value == "const":
                consts.append(self.parse_const())
            elif t.kind == "name" and t.value == "def":
                funcs.append(self.parse_func())
            else:
                self.err(f"unexpected {t.value or t.kind!r} at top level")
        return funcs, structs, consts

    def parse_type(self) -> str:
        t = self.expect("name")
        name = t.value
        if self.accept("op", "["):          # List[T] keeps its element type
            inner = self.parse_type()
            self.expect("op", "]")
            if name == "List":
                return f"List[{inner}]"
            return name
        return name

    def parse_sig(self, extern=False) -> Func:
        line = self.expect("name", "def").line
        name = self.expect("name").value
        self.expect("op", "(")
        params = []
        while not self.at("op", ")"):
            pname = self.expect("name").value
            self.expect("op", ":")
            ptype = self.parse_type()
            params.append(Param(pname, ptype))
            if not self.accept("op", ","):
                break
        self.expect("op", ")")
        ret = "Void"
        if self.accept("op", "->"):
            ret = self.parse_type()
        return Func(name, params, ret, [], line, extern)

    def parse_extern(self):
        self.expect("name", "extern")
        self.expect("str")                       # "C"
        if self.at("name", "link"):
            self.next()
            self.expect("str")
        self.expect("op", ":")
        self.expect("newline")
        self.expect("indent")
        out = []
        while not self.at("dedent") and not self.at("eof"):
            if self.accept("newline"):
                continue
            out.append(self.parse_sig(extern=True))
            self.accept("newline")
        self.expect("dedent")
        return out

    def parse_struct(self) -> Struct:
        line = self.expect("name", "struct").line
        name = self.expect("name").value
        self.expect("op", ":")
        self.expect("newline")
        self.expect("indent")
        fields = []
        while not self.at("dedent") and not self.at("eof"):
            if self.accept("newline"):
                continue
            fname = self.expect("name").value
            self.expect("op", ":")
            ftype = self.parse_type()
            fields.append(Param(fname, ftype))
            self.accept("newline")
        self.expect("dedent")
        return Struct(name, fields, line)

    def parse_const(self) -> Const:
        line = self.expect("name", "const").line
        name = self.expect("name").value
        self.expect("op", "=")
        expr = self.parse_expr()
        self.accept("newline")
        return Const(name, expr, line)

    def parse_func(self) -> Func:
        fn = self.parse_sig()
        self.expect("op", ":")
        fn.body = self.parse_block()
        return fn

    def parse_block(self):
        self.expect("newline")
        self.expect("indent")
        stmts = []
        while not self.at("dedent") and not self.at("eof"):
            if self.accept("newline"):
                continue
            stmts.append(self.parse_stmt())
        self.expect("dedent")
        return stmts

    # -- statements
    def parse_stmt(self):
        t = self.peek()
        if t.kind == "name":
            if t.value == "if":
                return self.parse_if()
            if t.value == "while":
                line = self.next().line
                cond = self.parse_expr()
                self.expect("op", ":")
                return Node("while", line, cond, self.parse_block())
            if t.value == "for":
                return self.parse_for()
            if t.value == "return":
                line = self.next().line
                expr = None
                if not self.at("newline"):
                    expr = self.parse_expr()
                self.accept("newline")
                return Node("return", line, expr)
            if t.value in ("break", "continue", "pass"):
                self.next()
                self.accept("newline")
                return Node(t.value, t.line)
            if t.value == "mut":
                line = self.next().line
                return self.parse_declare(line)
            if self.peek(1).kind == "op" and self.peek(1).value == ":=":
                return self.parse_declare(t.line)
            if (self.peek(1).kind == "op" and self.peek(1).value == ":"
                    and self.peek(2).kind == "name"
                    and self.is_annotated_decl()):
                return self.parse_declare(t.line)

        target = self.parse_expr()
        if self.at("op") and self.peek().value in ("=", "+=", "-=", "*=", "/="):
            op = self.next().value
            value = self.parse_expr()
            self.accept("newline")
            return Node("assign", t.line, target, value, op)
        self.accept("newline")
        return Node("exprstmt", t.line, target)

    def is_annotated_decl(self) -> bool:
        """Distinguish `x: Int := 0` from a plain expression statement."""
        j = self.i + 2
        depth = 0
        while j < len(self.toks):
            t = self.toks[j]
            if t.kind in ("newline", "eof"):
                return False
            if t.kind == "op":
                if t.value == "[":
                    depth += 1
                elif t.value == "]":
                    depth -= 1
                elif t.value == ":=" and depth == 0:
                    return True
                elif depth == 0 and t.value not in (".",):
                    return False
            j += 1
        return False

    def parse_declare(self, line):
        name = self.expect("name").value
        ann = None
        if self.accept("op", ":"):
            ann = self.parse_type()
        self.expect("op", ":=")
        expr = self.parse_expr()
        self.accept("newline")
        node = Node("declare", line, name, expr, False)
        node.text = ann or ""
        return node

    def parse_if(self):
        line = self.expect("name", "if").line
        cond = self.parse_expr()
        self.expect("op", ":")
        body = self.parse_block()
        node = Node("if", line, cond, body, None)
        cur = node
        while self.at("name", "elif"):
            self.next()
            econd = self.parse_expr()
            self.expect("op", ":")
            ebody = self.parse_block()
            nxt = Node("if", line, econd, ebody, None)
            cur.c = [nxt]
            cur = nxt
        if self.at("name", "else"):
            self.next()
            self.expect("op", ":")
            cur.c = self.parse_block()
        return node

    def parse_for(self):
        line = self.expect("name", "for").line
        var = self.expect("name").value
        self.expect("name", "in")
        self.expect("name", "range")
        self.expect("op", "(")
        first = self.parse_expr()
        second = None
        if self.accept("op", ","):
            second = self.parse_expr()
        self.expect("op", ")")
        self.expect("op", ":")
        body = self.parse_block()
        start, stop = (Node("num", line, "0"), first) if second is None else (first, second)
        n = Node("for", line, var, body)
        n.items = [start, stop]
        return n

    # -- expressions (precedence climbing)
    def parse_expr(self):
        return self.parse_or()

    def parse_or(self):
        left = self.parse_and()
        while self.at("name", "or"):
            line = self.next().line
            left = Node("binop", line, left, self.parse_and(), text="||")
        return left

    def parse_and(self):
        left = self.parse_not()
        while self.at("name", "and"):
            line = self.next().line
            left = Node("binop", line, left, self.parse_not(), text="&&")
        return left

    def parse_not(self):
        if self.at("name", "not"):
            line = self.next().line
            return Node("unop", line, self.parse_not(), text="!")
        return self.parse_cmp()

    def parse_cmp(self):
        left = self.parse_bitor()
        while self.at("op") and self.peek().value in ("==", "!=", "<", "<=", ">", ">="):
            op = self.next()
            left = Node("cmp", op.line, left, self.parse_bitor(), text=op.value)
        return left

    def parse_bitor(self):
        left = self.parse_bitxor()
        while self.at("op", "|"):
            line = self.next().line
            left = Node("binop", line, left, self.parse_bitxor(), text="|")
        return left

    def parse_bitxor(self):
        left = self.parse_bitand()
        while self.at("op", "^"):
            line = self.next().line
            left = Node("binop", line, left, self.parse_bitand(), text="^")
        return left

    def parse_bitand(self):
        left = self.parse_shift()
        while self.at("op", "&"):
            line = self.next().line
            left = Node("binop", line, left, self.parse_shift(), text="&")
        return left

    def parse_shift(self):
        left = self.parse_add()
        while self.at("op") and self.peek().value in ("<<", ">>"):
            op = self.next()
            left = Node("binop", op.line, left, self.parse_add(), text=op.value)
        return left

    def parse_add(self):
        left = self.parse_mul()
        while self.at("op") and self.peek().value in ("+", "-"):
            op = self.next()
            left = Node("binop", op.line, left, self.parse_mul(), text=op.value)
        return left

    def parse_mul(self):
        left = self.parse_unary()
        while self.at("op") and self.peek().value in ("*", "/", "%"):
            op = self.next()
            left = Node("binop", op.line, left, self.parse_unary(), text=op.value)
        return left

    def parse_unary(self):
        if self.at("op", "-"):
            line = self.next().line
            return Node("unop", line, self.parse_unary(), text="-")
        if self.at("op", "~"):
            line = self.next().line
            return Node("unop", line, self.parse_unary(), text="~")
        return self.parse_postfix()

    def parse_postfix(self):
        node = self.parse_primary()
        while True:
            if self.at("op", "("):
                line = self.next().line
                args = []
                while not self.at("op", ")"):
                    args.append(self.parse_expr())
                    if not self.accept("op", ","):
                        break
                self.expect("op", ")")
                call = Node("call", line, node)
                call.items = args
                node = call
            elif self.at("op", "."):
                line = self.next().line
                fieldname = self.expect("name").value
                node = Node("field", line, node, text=fieldname)
            else:
                return node

    def parse_primary(self):
        t = self.peek()
        if t.kind == "num":
            self.next()
            return Node("num", t.line, t.value)
        if t.kind == "str":
            self.next()
            return Node("strlit", t.line, t.value)
        if t.kind == "name":
            if t.value == "true":
                self.next()
                return Node("num", t.line, "1")
            if t.value == "false":
                self.next()
                return Node("num", t.line, "0")
            if t.value == "null":
                self.next()
                return Node("null", t.line)
            if t.value in KEYWORDS and t.value not in ("range",):
                self.err(f"unexpected keyword {t.value!r} in expression")
            self.next()
            return Node("name", t.line, t.value)
        if self.at("op", "("):
            self.next()
            e = self.parse_expr()
            self.expect("op", ")")
            return e
        self.err(f"unexpected {t.value or t.kind!r} in expression")


# --------------------------------------------------------------------------
# builtins (mirrors the L++ builtin surface the kernel profile keeps)
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
class Codegen:
    def __init__(self, funcs, structs, consts, fname="<module>"):
        self.funcs = {f.name: f for f in funcs}
        self.structs = {s.name: s for s in structs}
        self.consts = {c.name: c for c in consts}
        self.fname = fname
        self.out: list[str] = []
        self.scopes: list[dict] = []
        self.cur_ret = "Void"

    # -- types
    def ctype(self, t: str) -> str:
        if t.startswith("List"):
            return "void *"
        if t in PRIMITIVES:
            return PRIMITIVES[t]
        if t in self.structs:
            return f"{t} *"
        raise LppError(f"unknown type {t!r}", 0, self.fname)

    def declare(self, name, type_):
        self.scopes[-1][name] = type_

    def lookup(self, name):
        for s in reversed(self.scopes):
            if name in s:
                return s[name]
        if name in self.consts:
            return "Int"
        return None

    # -- expression types
    def type_of(self, n: Node) -> str:
        k = n.kind
        if k == "num":
            return "Int"
        if k == "strlit":
            return "Str"
        if k == "null":
            return "Ptr"
        if k == "name":
            t = self.lookup(n.a)
            if t is None:
                raise LppError(f"unknown identifier {n.a!r}", n.line, self.fname)
            return t
        if k in ("cmp",):
            return "Bool"
        if k == "binop":
            if n.text in ("&&", "||"):
                return "Bool"
            return self.type_of(n.a)
        if k == "unop":
            return "Bool" if n.text == "!" else self.type_of(n.a)
        if k == "field":
            base = self.type_of(n.a)
            st = self.structs.get(base)
            if not st:
                raise LppError(f"{base} is not a struct", n.line, self.fname)
            for f in st.fields:
                if f.name == n.text:
                    return f.type
            raise LppError(f"{base} has no field {n.text!r}", n.line, self.fname)
        if k == "call":
            callee = n.a
            if callee.kind != "name":
                raise LppError("only direct calls are supported", n.line, self.fname)
            name = callee.a
            if name == "list_get" and n.items:
                return self.elem_type(n.items[0])
            if name in self.structs:
                return name
            if name in self.funcs:
                return self.funcs[name].ret
            if name in BUILTINS:
                return BUILTINS[name][1]
            raise LppError(f"unknown function {name!r}", n.line, self.fname)
        raise LppError(f"cannot type expression {k}", n.line, self.fname)

    # -- expressions
    def expr(self, n: Node) -> str:
        k = n.kind
        if k == "num":
            return n.a
        if k == "strlit":
            return '"' + escape_c(n.a) + '"'
        if k == "null":
            return "0"
        if k == "name":
            if self.lookup(n.a) is None:
                raise LppError(f"unknown identifier {n.a!r}", n.line, self.fname)
            if n.a in self.consts:
                return n.a
            return cid(n.a)
        if k == "field":
            return f"{self.expr(n.a)}->{cid(n.text)}"
        if k == "unop":
            return f"({n.text}{self.expr(n.a)})"
        if k == "binop":
            if n.text == "+" and self.type_of(n.a) == "Str":
                return f"str_concat({self.expr(n.a)}, {self.expr(n.b)})"
            return f"({self.expr(n.a)} {n.text} {self.expr(n.b)})"
        if k == "cmp":
            lt = self.try_type(n.a)
            rt = self.try_type(n.b)
            if "Str" in (lt, rt) and n.text in ("==", "!="):
                eq = f"str_eq({self.expr(n.a)}, {self.expr(n.b)})"
                return eq if n.text == "==" else f"(!{eq})"
            return f"({self.expr(n.a)} {n.text} {self.expr(n.b)})"
        if k == "call":
            return self.call(n)
        raise LppError(f"cannot compile expression {k}", n.line, self.fname)

    def elem_type(self, n) -> str:
        t = self.try_type(n) or "List[Int]"
        if t.startswith("List[") and t.endswith("]"):
            return t[5:-1]
        return "Int"

    def try_type(self, n):
        try:
            return self.type_of(n)
        except LppError:
            return None

    def call(self, n: Node) -> str:
        name = n.a.a
        args = [self.expr(a) for a in n.items]
        if name in self.structs:
            st = self.structs[name]
            if len(args) != len(st.fields):
                raise LppError(
                    f"{name}() expects {len(st.fields)} fields, got {len(args)}",
                    n.line, self.fname,
                )
            return f"{name}__make({', '.join(args)})"
        if name in self.funcs:
            fn = self.funcs[name]
            if len(args) != len(fn.params):
                raise LppError(
                    f"{name}() expects {len(fn.params)} args, got {len(args)}",
                    n.line, self.fname,
                )
            target = fn.name if fn.extern else cname(name)
            return f"{target}({', '.join(args)})"
        if name == "list_get" and len(args) == 2:
            et = self.elem_type(n.items[0])
            raw = f"list_get({args[0]}, (int)({args[1]}))"
            if et in ("Int", "Bool"):
                return raw
            ct = self.ctype(et).strip()
            return f"(({ct})(long){raw})"
        if name in BUILTINS:
            cn, _ret, sig = BUILTINS[name]
            if len(args) != len(sig):
                raise LppError(
                    f"{name}() expects {len(sig)} args, got {len(args)}",
                    n.line, self.fname,
                )
            cast = []
            for a, t in zip(args, sig):
                cast.append(f"(int)(long)({a})" if t == "Int" else a)
            return f"{cn}({', '.join(cast)})"
        raise LppError(f"unknown function {name!r}", n.line, self.fname)

    # -- statements
    def emit(self, indent, text):
        self.out.append("    " * indent + text)

    def block(self, stmts, indent):
        self.scopes.append({})
        for s in stmts:
            self.stmt(s, indent)
        self.scopes.pop()

    def stmt(self, n: Node, ind):
        k = n.kind
        if k == "declare":
            name, expr, _mut = n.a, n.b, n.c
            t = n.text or self.type_of(expr)
            if t == "Void":
                raise LppError(f"cannot bind void result to {name!r}", n.line, self.fname)
            code = self.expr(expr)
            self.declare(name, t)
            decl = self.ctype(t)
            sep = "" if decl.endswith("*") else " "
            self.emit(ind, f"{decl}{sep}{cid(name)} = {code};")
        elif k == "assign":
            target, value, op = n.a, n.b, n.c
            if target.kind not in ("name", "field"):
                raise LppError("invalid assignment target", n.line, self.fname)
            if target.kind == "name" and self.lookup(target.a) is None:
                raise LppError(
                    f"{target.a!r} is not declared (use ':=' to declare)", n.line, self.fname
                )
            self.emit(ind, f"{self.expr(target)} {op} {self.expr(value)};")
        elif k == "if":
            self.emit(ind, f"if ({self.truth(n.a)}) {{")
            self.block(n.b, ind + 1)
            if n.c:
                self.emit(ind, "} else {")
                self.block(n.c, ind + 1)
            self.emit(ind, "}")
        elif k == "while":
            self.emit(ind, f"while ({self.truth(n.a)}) {{")
            self.block(n.b, ind + 1)
            self.emit(ind, "}")
        elif k == "for":
            var = n.a
            start, stop = n.items
            self.scopes.append({var: "Int"})
            tmp = f"__stop_{n.line}_{len(self.out)}"
            self.emit(ind, f"{{ int {tmp} = {self.expr(stop)};")
            cv = cid(var)
            self.emit(ind, f"for (int {cv} = {self.expr(start)}; {cv} < {tmp}; {cv}++) {{")
            self.block(n.b, ind + 1)
            self.emit(ind, "} }")
            self.scopes.pop()
        elif k == "return":
            if n.a is None:
                self.emit(ind, "return;")
            else:
                self.emit(ind, f"return {self.expr(n.a)};")
        elif k == "break":
            self.emit(ind, "break;")
        elif k == "continue":
            self.emit(ind, "continue;")
        elif k == "pass":
            self.emit(ind, ";")
        elif k == "exprstmt":
            self.emit(ind, f"(void)({self.expr(n.a)});")
        else:
            raise LppError(f"cannot compile statement {k}", n.line, self.fname)

    def truth(self, n):
        return self.expr(n)

    # -- module
    def generate(self) -> str:
        w = self.out.append
        w("/* Generated by SamarOS lppc — do not edit.")
        w(" * Source of truth: the .lpp files in SamarOS/kernel/src")
        w(" */")
        w('#include "kernel_api.h"')
        w("")

        for s in self.structs.values():
            w(f"typedef struct {s.name} {s.name};")
        w("")
        for s in self.structs.values():
            w(f"struct {s.name} {{")
            for f in s.fields:
                ct = self.ctype(f.type)
                sep = "" if ct.endswith("*") else " "
                w(f"    {ct}{sep}{cid(f.name)};")
            w("};")
            args = ", ".join(
                f"{self.ctype(f.type)}{'' if self.ctype(f.type).endswith('*') else ' '}{cid(f.name)}"
                for f in s.fields
            ) or "void"
            w(f"static {s.name} *{s.name}__make({args}) {{")
            w(f"    {s.name} *self = ({s.name} *)lpp_alloc(sizeof({s.name}));")
            for f in s.fields:
                w(f"    self->{cid(f.name)} = {cid(f.name)};")
            w("    return self;")
            w("}")
            w("")

        for c in self.consts.values():
            self.scopes = [{}]
            w(f"#define {c.name} ({self.expr(c.expr)})")
        w("")

        for fn in self.funcs.values():
            if fn.extern:
                params = ", ".join(
                    f"{self.ctype(p.type)}{'' if self.ctype(p.type).endswith('*') else ' '}{cid(p.name)}"
                    for p in fn.params
                ) or "void"
                w(f"extern {self.ctype(fn.ret)} {fn.name}({params});")
        w("")

        for fn in self.funcs.values():
            if fn.extern:
                continue
            w(self.signature(fn) + ";")
        w("")

        for fn in self.funcs.values():
            if fn.extern:
                continue
            self.scopes = [{p.name: p.type for p in fn.params}]
            self.cur_ret = fn.ret
            w(self.signature(fn) + " {")
            self.block(fn.body, 1)
            if fn.ret == "Void":
                self.emit(1, "return;")
            else:
                self.emit(1, "return 0;")
            w("}")
            w("")

        return "\n".join(self.out) + "\n"

    def signature(self, fn: Func) -> str:
        params = ", ".join(
            f"{self.ctype(p.type)}{'' if self.ctype(p.type).endswith('*') else ' '}{cid(p.name)}"
            for p in fn.params
        ) or "void"
        ret = self.ctype(fn.ret)
        sep = "" if ret.endswith("*") else " "
        storage = "" if fn.name == "main" else "static "
        return f"{storage}{ret}{sep}{cname(fn.name)}({params})"


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


def main():
    if len(sys.argv) < 3:
        print(__doc__)
        return 1
    out_path = sys.argv[1]
    sources = sys.argv[2:]

    funcs, structs, consts = [], [], []
    for path in sources:
        with open(path) as fh:
            src = fh.read()
        toks = tokenize(src, path)
        f, s, c = Parser(toks, path).parse_module()
        funcs.extend(f)
        structs.extend(s)
        consts.extend(c)

    seen = {}
    for fn in funcs:
        if fn.name in seen and not fn.extern:
            raise LppError(f"duplicate definition of {fn.name!r}", fn.line, "lppc")
        seen[fn.name] = fn

    gen = Codegen(funcs, structs, consts)
    code = gen.generate()
    with open(out_path, "w") as fh:
        fh.write(code)

    lines = sum(1 for p in sources for _ in open(p))
    print(
        "lppc: %d L++ files, %d lines -> %s (%d structs, %d functions)"
        % (len(sources), lines, out_path, len(gen.structs),
           len([f for f in funcs if not f.extern]))
    )
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except LppError as e:
        print(f"lppc error: {e}", file=sys.stderr)
        sys.exit(1)
