# Compiler developer notes

**Current as of 2026-07-30.**

## Important files

| File | Purpose |
|---|---|
| `src/frontend/lexer.rs` | tokens, literals, indentation |
| `src/frontend/parser.rs` | recursive descent parser |
| `src/frontend/ast.rs` | syntax tree |
| `src/analysis/semantic.rs` | scopes and binding resolution |
| `src/analysis/typecheck.rs` | types and inference |
| `src/analysis/monomorph.rs` | generic specialization |
| `src/analysis/cyclebreak.rs` | static ownership-cycle edge demotion |
| `src/mir/lower.rs` | AST to MIR lowering |
| `src/mir/escape_solver.rs` | single MIR ownership fact |
| `src/mir/pass_arc.rs` | retain/release and destructor cleanup |
| `src/mir/pass_escape.rs` | stack promotion |
| `src/backend/cranelift/` | default object backend |
| `src/backend/llvm.rs` | optional LLVM object backend |
| `src/bin/lpp-link.rs` | direct linker |
| `src/builtins.rs` | builtin signatures |

The old AST escape analyzer and Turbo mode are not part of the current tree.

## Adding a feature

Most language features touch:

1. AST;
2. lexer/parser;
3. semantic resolver;
4. type checker;
5. MIR lowering;
6. every backend that supports the new MIR form;
7. runtime symbols if the feature crosses the runtime boundary.

Do not edit `lpp-link` for an ordinary language feature unless object format or
relocation support changes.

## Ownership rules

Ownership is solved over MIR using `Frame < Owned < Shared`. Do not introduce a
second escape analysis. A new pointer-bearing MIR rvalue must be classified in
the exhaustive solver match and tested through both backends.

Arena-backed recursive nodes must preserve region lifetime and static cycle-break
invariants. Stack payloads must never be sent to ARC header functions.

## Vector rules

Explicit `VectorI64x2` builtins are ordinary value MIR. They must not be treated
as managed pointers. If a new vector operation is added, implement it in both
Cranelift and LLVM or return a clear unsupported-backend diagnostic.

Do not claim automatic vectorization from a scalar benchmark without checking the
object for SIMD instructions.

## Validation

```sh
cargo test --release -j1
sh tests/run_aot_parity.sh
sh scripts/check_safety_mission.sh
(cd packages/lppsqlite && sh run-tests.sh)
(cd packages/compresslpp && sh run-tests.sh)
```

For LLVM:

```sh
LPP_LLVM_CC=/usr/bin/clang tests/run_llvm_smoke.sh
```

Use ASan/UBSan for ownership changes and TSan for thread/region changes.
