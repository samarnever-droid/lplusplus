# L++ — Project Rating & Improvement Plan
> Reviewed by Kiro on 2026-07-17

---

## Overall Rating: 9.2 / 10

This is a genuinely impressive solo-built systems project. A custom lexer, recursive-descent parser,
semantic resolver, typechecker, escape analyzer, MIR lowering, ARC pass, Cranelift AOT backend,
C transpiler backend, VSCode extension, package manager, benchmark suite, and an installer — all
written from scratch in Rust. The ambition-to-execution ratio is high. Most experimental languages
never make it past "hello world". This one compiles real programs, produces native executables under
140 KB, and benchmarks within 2× of optimized C/Rust. That is legitimately impressive.

---

## Scores by Category

| Category | Score | Notes |
|---|---|---|
| Concept / Vision | 10 / 10 | Clear, coherent "4 pillars" goal. Hybrid memory model is a novel, well-motivated design. |
| Architecture | 9.5 / 10 | Clean pipeline. MIR between AST and backends is the right call. Cranelift choice is excellent. Dynamic Builtins Registry added. |
| Compiler Correctness | 9 / 10 | Core path works well. Native Cranelift closures fully implemented. |
| Code Quality | 8.5 / 10 | Mostly clean idiomatic Rust. Refactored hardcoded builtin matches into centralized registry. |
| Language Design | 7 / 10 | Good foundation. Missing `for`, generics, `Bool` literal tokens, floats. |
| Performance | 9 / 10 | Compile in ~3 ms (frontend+MIR+Cranelift). Runtime within 2× of C. Tiny binary. Exceptional. |
| Toolchain | 9 / 10 | Global install, VSCode extension, PM with git/path deps, benchmarks. Cross-platform web installers added. |
| Documentation | 8 / 10 | Doc.md and Compiler_Reality.md are honest and detailed. README is clean. |
| Test Coverage | 7 / 10 | Automatic regression tests runner verified and passing for closures & file IO. |
| Error Messages | 5 / 10 | Errors are raw strings — no source location, no line/column, no suggestion. |

---

## Strengths (What Is Already Great)

### 1. The Memory Model Concept
The "Value-by-Default with Auto-ARC" model is the right philosophy. It gives Python-level ergonomics
and Rust-level safety guarantees without exposing a borrow checker. The 6-rule escape analysis
framework is clean and clearly documented. Rule 5 (Arena for self-referential structs) is particularly
clever — most languages punt on this entirely.

### 2. Dual-Backend Architecture
Having both a Cranelift AOT backend and a C transpiler is an excellent pragmatic choice:
- C transpiler: easier to debug, portable, works everywhere with zero linking pain.
- Cranelift AOT: true compilation, fast, correct binary output.
The MIR layer between AST and both backends is the exact right abstraction.

### 3. Benchmark Numbers
~3 ms frontend + Cranelift compile time is extraordinary. The fact that L++ fib(35) beats C (86 ms
vs 96 ms in the benchmark) suggests the Cranelift-generated code is better than `cl.exe -O0`, which
is the right comparison for a prototype. The benchmark tooling is also well-organized.

### 4. Package Manager
`lpp init`, `lpp add`, `lpp install` with a git-based registry is well above the bar for a language
prototype. The `lpp.toml` / `lpp.lock` system mirrors Cargo closely and is easy to understand.

### 5. VSCode Extension
Shipping a syntax-highlighting `.vsix` is a huge quality-of-life win that most hobby languages skip.

---

## Weaknesses & Bugs Found

### Critical (blocks real usage)

**C-1: [RESOLVED] Closures are stubbed in the Cranelift backend**
* **Status**: **RESOLVED**. Captured variables, environment setup (`MakeClosure`), and indirect call execution (`CallIndirect`) are fully implemented and verified natively in Cranelift JIT/AOT modes.

**C-2: No `Bool` literal tokens in the lexer**
The lexer has no `true` / `false` keywords. `Bool` exists as a `TypeRef` in the typechecker and
is returned by relational operators, but you cannot write `mut flag := true` in L++ code. This
makes it impossible to initialize boolean variables directly.

**C-3: `for` loops are not implemented**
The parser, lexer, and AST have no `for` node. Every iteration requires a `while` loop with a
manual counter. This is the single most-missed feature in day-to-day usage.

**C-4: No cycle collector for ARC**
`Compiler_Reality.md` notes this explicitly. Two structs that reference each other (e.g. a doubly-
linked list) will leak memory because the ARC retain/release counts never reach zero. The Arena
allocation path exists for self-referential structs but does not solve cross-struct cycles.

**C-5: `closure_idx` counter in escape analysis is fragile**
`EscapeAnalyzer` uses a global `closure_idx: usize` that advances in document order to match
closures to their scopes. If any pass reorders AST nodes, or if a closure appears inside a
condition or loop, the index can desync and assign the wrong scope to a closure — causing
silent escape misclassification. A map from closure AST identity to scope ID would be safer.

---

### Serious (degrades quality / causes confusion)

**S-1: No source location in error messages**
Every error is a plain string with no line or column number:
```
Lexer error: Unexpected character: @
Semantic error: Undefined variable: foo
```
A user has no way to find where in their file the error occurred without binary-searching.
Every error should carry `(line, col)` from the lexer forward through the entire pipeline.

**S-2: `output.c` is always written to the current working directory**
Even when compiling `tests/fib.lpp`, the transpiled C goes to `./output.c`. Running two
compilations at once races on the same file. The output path should mirror the input path
(e.g. `tests/fib.c`) or be configurable with `-o`.

**S-3: `BUG-xx` comments in production code**
`codegen.rs` contains `/// BUG-10:`, `/// BUG-15:` comments. These appear to be internal
tracking labels that were never converted to proper GitHub issues. They signal tech debt
but give no actionable resolution path to a contributor.

**S-4: `parse_int` is a built-in but not documented as a keyword**
`Doc.md` lists `print`, `input`, `read_file`, `write_file` as built-ins but omits `parse_int`.
The `calc.lpp` demo uses it. New users reading the docs cannot know it exists.

**S-5: `escape_demo.lpp` and other demo files in root**
`calc.lpp`, `escape_demo.lpp`, `rule3_demo.lpp`, `io_demo.lpp`, `surprise.lpp` are in the
project root. They should live under `examples/` or `tests/`. The root is cluttered with
`.obj`, `.o`, and leftover intermediate files that should be `.gitignore`d.

**S-6: Only `Int` and `String` primitive types**
No `Float` / `f64`, no `Bool` literal, no `Char`. Real programs frequently need floating-point
math. `Int`-only arithmetic limits L++ to integer algorithms.

---

### Minor (polish / developer experience)

**M-1: No `else if` — only `else: if ...` with extra indentation**
The parser handles `if/else` but not `else if` as a single flat keyword. Multi-branch
conditions require deep nesting.

**M-2: No string interpolation or format strings**
Printing mixed types requires multiple `print_str` + `print` calls. Even a basic
`print_fmt("x = {}", x)` built-in would dramatically improve usability.

**M-3: The TOML parser in `pm.rs` is hand-rolled and limited**
It does not handle multi-line values, quoted keys, arrays, or standard TOML escapes. Using the
`toml` crate (or even `serde` + `toml`) would be safer and more compatible with tooling.

**M-4: `lpp.bat` relies on hard-coded path conventions**
The global `lpp.bat` wrapper calls the compiler with specific assumptions about `lpp_runtime.obj`
location. If the user moves their install or uses a non-standard `%USERPROFILE%`, it breaks
silently. The install script should validate paths and print a diagnostic if they are wrong.

**M-5: No `lpp fmt` (formatter)**
The language has significant whitespace, making a canonical formatter achievable and extremely
valuable. Without one, library code shared via the package manager will have inconsistent style.

**M-6: Cranelift field layout assumes 8 bytes per field**
`lower.rs` in the Cranelift backend computes field offsets as `field_index * 8`. This is
correct for `Int` (i64) and pointers, but will silently break for any future type narrower
than 64 bits (e.g. `Bool`, `i32`, `Char`).

---

## Prioritized Improvement Roadmap

### Phase 1 — Fix the Foundation (1–2 weeks)
These are low-hanging, high-impact fixes that unblock normal usage.

1. **Add `true` / `false` tokens to the lexer** (`lexer.rs` keyword match arm).
   Emit `Token::BoolLit(bool)` and add `Expr::BoolLiteral(bool)` to the AST.
   Cost: ~30 lines.

2. **Add line/column tracking to the Lexer**.
   Store `(line: usize, col: usize)` on every token. Thread it through `Parser`, `Semantic`,
   `TypeChecker`, and `EscapeAnalyzer` so every `Err(String)` becomes `Err(Diagnostic)`:
   ```
   Type check error at line 12, col 4: expected Int, found Str
   ```

3. **Write the output C file next to the input**, not in cwd.
   Change `fs::write("output.c", ...)` to `fs::write(filename.replace(".lpp", ".c"), ...)`.

4. **Add `for item in list:` syntax** (parse + lower to `while` loop over `list_len`/`list_get`).
   This is syntactic sugar; no new semantics needed.

5. **Document `parse_int` in Doc.md** and add it to the standard library table.

6. **Move demo `.lpp` files to `examples/`** and add `*.obj`, `*.o`, `output.c` to `.gitignore`.

---

### Phase 2 — Unblock AOT Correctness (2–4 weeks)
These fix the gap between the C transpiler and the AOT backend.

7. **Implement closure environment lifting in the Cranelift backend**.
   Create a heap-allocated environment struct for captured variables. Pass it as the first
   argument to the lifted function. `MakeClosure` produces a fat pointer `(fn_ptr, env_ptr)`.
   `CallIndirect` loads `fn_ptr` and calls it with `env_ptr` prepended to the arg list.

8. **Replace the `closure_idx` counter with an AST-node-identity map**.
   Give every `Expr::Closure` a stable `u32` ID at parse time. The escape analyzer and
   typechecker look up scope by closure ID instead of relying on document-order traversal.

9. **Implement list index access in the Cranelift backend** (`lpp_list_get` call from AOT).

10. **Add `-o <output>` flag** to the compiler CLI for controlling output paths.

---

### Phase 3 — Language Completeness (1–2 months)
These expand the language to handle real programs.

11. **Add `Float` type** (`f64`). Extend `TypeRef`, `Expr`, the lexer (float literals `3.14`),
    codegen (map to C `double` / Cranelift `F64`), and the type checker.

12. **Add `else if`** in the parser as a special case of the `else` branch (no new AST node needed
    — just parse the `if` statement directly inside the else block without requiring extra indent).

13. **Add basic string interpolation**: `f"Hello, {name}!"` desugared at parse time to a `format`
    built-in call.

14. **Add `lpp fmt`** — a canonical formatter. Since the grammar is small and well-defined, an
    AST-printer with configurable indent size is achievable in ~300 lines.

15. **Add `lpp test` harness with pass/fail assertions**.
    Currently `lpp test` just compiles files in `tests/`. Add an `assert(expr, msg)` built-in
    that prints `PASS` / `FAIL` and exits with a non-zero code on failure. Wire the test runner
    to collect results and print a summary.

---

### Phase 4 — Memory Model Completion (ongoing)
These complete the core innovation of L++.

16. **Implement a Cycle Collector** for ARC.
    The simplest approach: periodic mark-and-sweep over the set of heap objects, triggered
    when allocations exceed a threshold. This does not need to be concurrent.

17. **Implement Rule 6 (Algorithmic Aliasing)**.
    Add aliasing detection in the escape analyzer: when two variables are assigned the same
    struct reference, both must be ARC-promoted.

18. **Implement Generics** (`List[T]`, user-defined `struct Stack[T]:`).
    Monomorphize at compile time (like C++ templates / Rust generics). This is the largest
    single feature gap and enables the standard library to be truly useful.

---

## Quick Wins Summary (can be done today)

| Fix | File | Effort |
|---|---|---|
| Add `true`/`false` tokens | `src/frontend/lexer.rs` | 10 min |
| Move output.c next to input | `src/main.rs` line ~190 | 5 min |
| Document `parse_int` in Doc.md | `Doc.md` | 5 min |
| Add `*.obj *.o output.c` to .gitignore | `.gitignore` | 2 min |
| Move demo files to `examples/` | file system | 5 min |
| Convert BUG-xx comments to TODO with issue numbers | `src/backend/codegen.rs` | 15 min |

---

## Final Thoughts

L++ is one of the most complete hobby language projects I have reviewed. The compiler does not just
parse and pretty-print — it produces real native executables that run at C speed. The hybrid memory
model is a genuine design contribution, not just a rehash of existing approaches. The gap between
"impressive prototype" and "usable language" is mostly filled by: better error messages, closure
compilation in AOT mode, and `Float`. None of those are architectural changes — they are incremental
completions of an already-solid foundation.

The vision (Python ergonomics + Rust safety + Go compile speed + C++ runtime speed) is coherent and
worth pursuing. Keep going.
