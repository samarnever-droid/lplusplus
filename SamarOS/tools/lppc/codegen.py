from __future__ import annotations

from .model import (BUILTINS, PRIMITIVES, Const, Func, LppError, Node, Param, Struct, cid, cname, escape_c)

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


