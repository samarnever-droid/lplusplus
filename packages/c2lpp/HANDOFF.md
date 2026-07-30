# c2lpp handoff — Phase 1 complete, Phase 2 in progress

**Date:** 2026-07-30  
**Patch base:** `5873ce7b781cf4c3d89c11ab418285a868aed164`  
**Package version:** `c2lpp 0.2.0`

## Read this first

The user requires a **pure-L++ translation of the complete SQLite
implementation**, not merely SQLite C bindings. Do not describe header bindings
as a completed pure-L++ SQLite conversion.

Only publish/deliver a patch when the included increment works end to end:

1. generated L++ parses and type-checks;
2. both required native paths compile where applicable;
3. generated code executes;
4. output is compared against native C or SQLite;
5. unsupported C is explicit, never silently guessed.

Intermediate parser/translator work may be backed up privately, but must not be
presented as a completed public patch.

---

## Current completed work

### Phase 1 — complete for the declared tier-1 ABI package boundary

`c2lpp` is a pure-L++ package under `packages/c2lpp` and is registered in
`registry/index.json`.

Implemented and tested:

- optional `cc -E` preprocessing;
- line-marker filtering so declarations from included system headers do not
  pollute the generated library API;
- C/C++ comment handling;
- integer and hexadecimal macro constants;
- multiline prototypes;
- callback-aware parameter splitting;
- integer, Float, Bool, string, pointer and pointer-to-pointer ABI mapping;
- opaque handle/callback representation;
- L++ reserved-name sanitization;
- conservative skips for C varargs and unsupported by-value ABI;
- generated `bindings.lpp`, `lpp.toml`, README and dependency report;
- safe shell-atom checks for external preprocessing.

Measured real-header results in the current environment:

- `/usr/include/sqlite3.h`: **270** declarations generated, bindings type-check,
  native `sqlite3_libversion()` package links and runs;
- `/usr/include/zlib.h`: **73** declarations generated, bindings type-check,
  native `zlibVersion()` package links and runs.

### Phase 2 — working audited subset

Implemented and tested C-to-pure-L++ translation for:

- scalar function signatures and parameters;
- scalar locals and initialization;
- arithmetic, calls, assignment and compound assignment;
- `if`/`else` and `while`;
- canonical ascending `for` loops translated to `range`;
- increment/decrement;
- fixed local scalar arrays translated to checked L++ lists;
- checked array reads and `list_set` writes;
- returns;
- unsupported-code comments;
- unsupported construct count/report and strict mode;
- native C versus translated L++ output-equivalence tests.

The supported fixture compiles as C and translated L++, and both produce:

```text
45
45
7
10
24
```

The unsupported pointer/goto fixture emits three quarantine markers and the
generated file still type-checks.

---

## Validation command

```sh
LPP=/path/to/lpp sh packages/c2lpp/run-tests.sh
```

Expected result:

```text
PASS SQLite header -> L++ native package
PASS zlib header -> L++ native package
PASS Phase 2 scalar C -> pure L++ translation/native equivalence smoke
PASS Phase 2 unsupported-code quarantine/report (3 markers)
PASS system SQLite header/native smoke (... declarations)
PASS system zlib header/native smoke (... declarations)
c2lpp tests: PASS
```

System SQLite/zlib checks are conditional when development headers/libraries are
not installed.

---

## What is not complete

The complete SQLite amalgamation is not translated. The tested SQLite 3.46.1
amalgamation has approximately:

```text
257,679 source lines
3,242 #define lines
1,053 lines containing goto
206 switch occurrences
1,984 for occurrences
942 while occurrences
50 union occurrences
```

These raw inventory numbers include conditional/comment noise, but show why the
line translator cannot honestly translate whole SQLite.

Major missing requirements:

- C tokenizer with source provenance;
- declaration/type graph and typedef resolution;
- struct, nested struct, bitfield and union layouts;
- pointer provenance and pointer arithmetic;
- borrowed/owned/mutable buffer representation;
- lvalue/place lowering;
- macro expansion provenance;
- function-pointer tables and callback lifetime policy;
- CFG construction and `goto` conversion;
- `switch`, fallthrough and labels;
- varargs;
- allocator pairing, realloc and lifetime transfer;
- setjmp/longjmp quarantine or transformation;
- atomics and threading semantics;
- SQLite OS/VFS/file-locking layer;
- differential SQLite database/query test corpus.

Do not try to solve these with string replacements.

---

## Recommended next implementation order

1. **Whole-source audit mode**
   - Parse the entire amalgamation and produce deterministic counts and sampled
     unsupported locations.
   - Fail if source lines disappear without classification.

2. **Normalized C IR**
   - Tokens with file/line/column.
   - Types, declarations, expressions, statements and labels.
   - Stable serialized form so parsing and emission are separately testable.

3. **CFG before source emission**
   - Build basic blocks for every function.
   - Resolve labels/gotos/switch edges.
   - Emit structured L++ only after reducibility analysis; quarantine the rest.

4. **C compatibility memory model**
   - Define pointer `(allocation, offset, provenance)` semantics.
   - Use checked buffers for translated memory access.
   - Never represent a C pointer as an ordinary ARC-owned L++ object without a
     proof/policy.

5. **Translate SQLite by subsystem**
   - integer/varint and byte utilities;
   - hash/string helpers;
   - memory allocator layer;
   - parser/tokenizer;
   - record/B-tree/pager;
   - VDBE;
   - SQL frontend;
   - VFS/OS boundary last.

6. **Differential validation**
   - Compile native C SQLite and translated L++ SQLite from the same source
     revision.
   - Compare database files, `integrity_check`, SQL output, error codes and
     teardown under sanitizers.

---

## Compiler-edit rules

If c2lpp exposes a missing L++ capability and the compiler must be changed:

1. Pull the newest remote before editing.
2. Keep compiler changes in a separate patch from package changes.
3. Update parser, typechecker, MIR, Cranelift, LLVM and all supported runtimes.
4. Run Cargo tests, AOT parity, LLVM smoke, feature tests, safety mission,
   lppsqlite and compresslpp.
5. Run host/direct and ASan/UBSan; use TSan for threading/runtime changes.
6. Do not call syntax-only work complete.
7. Do not push without explicit permission.

## Package-edit rules

- Keep the converter itself pure L++.
- External clang/cc may preprocess or export an AST, but policy, normalized IR
  validation and L++ emission remain in the package.
- Every accepted construct needs a C-vs-L++ executable equivalence test.
- Every rejected construct needs a stable diagnostic test.
- Never remove tests to make a gate pass.
- Never claim whole SQLite until the translated implementation runs without
  linking the SQLite C library.

---

## Patch and workspace delivery

The package patch should apply to the base commit above with `patch -p1`.
The standalone development copy is `/home/user/c2lpp` when present. Generated
packages, downloaded SQLite amalgamations, executables, objects and build
folders must not be included in the patch.
