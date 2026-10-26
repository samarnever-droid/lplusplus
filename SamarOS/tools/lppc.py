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

from lppc.codegen import Codegen
from lppc.lexer import tokenize
from lppc.model import LppError
from lppc.parser import Parser


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
