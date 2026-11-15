from __future__ import annotations

from typing import Optional

from .model import KEYWORDS, Const, Func, LppError, Node, Param, Struct, Tok

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
