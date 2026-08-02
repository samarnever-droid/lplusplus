# c2lpp handoff — Phase 1 complete, Phase 2 in progress

**Date:** 2026-07-31
**Current continuation base:** `7f8902412ecf447519b9908af51b9985ee63dd84`
**Upstream package commit:** `98e1644f0af385c7c05292553d2ed8a62ba6b01b`
**Package version:** `c2lpp 0.36.0`

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

### Whole-input audit and multi-file dependency inventory

The continuation adds a pure-L++ checked-buffer scanner and project manifest.
It proves lexical byte conservation, retains file/offset/line/column provenance,
resolves local/system include edges, and emits stable reason codes. It does not
claim those constructs are translated.

Pinned SQLite 3.46.1 results:

```text
source bytes:             9,089,564
covered bytes:            9,089,564
tokens:                   1,638,590
unknown bytes:            0
goto tokens in C code:    975
switch tokens in C code:  201
union tokens in C code:   35
whole_translation_complete=0
```

The optimized scan itself completed in 0.304 seconds in the handoff sandbox;
the full package gate, including download/checksum and all earlier tests,
completed in about four seconds. Timing is informational, while byte/hash and
reason-code checks are test assertions.

### Versioned JSON configuration and typed normalized-IR vertical slice

The executable now reads a strict `c2lpp.json` project file. Conversion settings
are no longer read from `C2LPP_*` environment variables. Schema v1 rejects
unknown/duplicate keys and value type mismatches, validates cross-field policy,
and emits deterministic `c2lpp.config.normalized.json` output.

JSON `mode: "translate-ir"` provides the first token/typed-IR/emitter path.
Implemented and executable:

- on-demand checked-buffer tokens with exact source spans;
- scalar C type records and function/local symbol tables;
- precedence-aware unary and binary expressions;
- typed function calls with arity and argument checking;
- scalar locals, assignments, compound assignments and returns;
- stable `c2lpp-normal-ir-v1` serialization before source emission;
- function-atomic unsupported quarantine and parser recovery;
- generated L++ type-check/build/execute and native C output equivalence.

### Pure-L++ compatibility lowering foundations

The continuation also adds separately executable foundations for semantics that
the parser does not yet accept:

- `CPtr` with context/allocation identity, bounds, generation, mutability and
  element width;
- flattened `CPlace` integer/bitfield lvalues and `CPointerPlace` pointer slots
  with indexing, nested fields, address/dereference, compound updates, swap,
  stale-target validation and overlap-safe copy;
- checked allocation/reallocation/free, pointer arithmetic, integer memory,
  memory-copy/move/set and C strings;
- raw, explicitly destroyed SysV x86-64 struct/union/bitfield layout tables;
- zeroed hidden global/static storage with initialization states;
- checked CFG block state machine for switch, fallthrough, goto and return.

Native C differential fixtures pass for each successful model. Out-of-bounds,
use-after-free, double-free, interior-free and readonly writes terminate with
stable diagnostics. All successful compatibility fixtures pass ASan+UBSan with
LeakSanitizer enabled. An earlier managed `List[Custom]` layout representation
was discarded after the sanitizer gate found leaks.

### Integrated strict no-binding native profile

JSON `mode: "native"` now connects those foundations for one fail-closed grammar.
`fixtures/native_profile.c` exercises:

- `typedef struct` and integer bitfield declarations;
- scalar zero/init globals plus a pointer-global relocation dependency;
- `calloc`, `free`, index/member, address-of, `(*p).field` and `p->field`;
- canonical loop;
- switch, fallthrough, goto cleanup and return;
- allocation/free pairing and no pointer escape.

Generated output contains no extern block, no native link directive and no C
source/header runtime. It compiles, runs, matches native C and passes
ASan/UBSan/LeakSanitizer. Unsupported input fails without falling back to
bindings.

This integrates only native profile v1, not arbitrary C. General declaration
and CFG construction, function-pointer graphs, all ABI targets and SQLite
subsystems remain incomplete.

### Expanded no-binding frontend profile v2

JSON `mode: "frontend"` additionally integrates, for one strict grammar:

- macro physical/expansion provenance;
- forward/incomplete struct typedef and callback typedef;
- nested aggregate and fixed-array declaration/layout;
- const string and integer-array globals;
- ordinary and variadic prototypes represented without extern emission;
- array/nested/pointer places;
- automatic canonical-loop plus switch/fallthrough/goto lowering;
- allocation call graph, balanced free and no-escape analysis.

The profile produces `197, 163, 143` in both native C and generated pure L++ and
passes ASan/UBSan/LeakSanitizer. It is not a general C frontend.

### General translation-unit partitioner

JSON `mode: "tu-graph"` accepts arbitrary top-level declaration order and emits
spanned records for typedefs, aggregate definitions, globals, prototypes and
function bodies. It recognizes function-pointer and variadic declarations.

The first token-object implementation was killed by memory pressure on SQLite;
it was replaced with a raw-byte scanner and raw growing graph buffer. The pinned
active preprocessed SQLite gate now completes in roughly 0.1–0.2 seconds and
reports:

```text
external declarations:       4430
typedef declarations:         242
aggregate declarations:       209
globals:                       96
prototypes:                  1355
function definitions:       2528
function-pointer declarations: 180
variadic declarations:         35
unknown declarations:           0
```

This is 100% top-level partition coverage, not 95% semantic conversion.

Two follow-up general passes now report:

```text
base-type families resolved: 4430/4430
function bodies balanced:    2528/2528
statements partitioned:       45222
basic-block candidates:       20955
```

An automatic semantic sweep now sends bounded bodies through the typed parser.
It emits 66/2,528 SQLite functions (2.61%) as one pure-L++ module that type-checks
with zero extern/native links. Conditional lowering covers lazy returns, local
initializers and ordinary/compound assignments across Int, Bool, Float and
pointer/null branches. Pointer logical operations emit explicit short-circuit
null tests. Character literals become C integer constants. Compile-time `sizeof`
supports primitive, pointer, aggregate and non-evaluated dereference forms;
parameter-driven aggregate layouts retain priority and one extra demand slot is
reserved for sizeof-only types. Pointer depth two now lowers through ABI
pointer-slot loads/stores and supports `**p`, `p[i]` and out-slot assignment.
One-dimensional array parameter syntax decays to `CPtr`; multidimensional arrays
and pointer depth three remain fail-closed. Six indirection functions match native
C and pass sanitizers. Braced do/while, empty statements and unbraced if/else call
statements are executable; do-loop continue remains fail-closed. Direct typed
mode covers do/while while the memory-bounded SQLite selector stays at 953
signatures, 1,017 eligible bodies and 66 accepted functions.

Two further parser-completeness additions (c2lpp 0.29.0) extend what an eligible
body may contain:

- **Uninitialized pointer locals** (`int *p;`) now default to null instead of
  being rejected with `C2-PLACE-UNINITIALIZED-POINTER`. This is sound — a null
  `CPtr` is a valid, safe value in the pure-L++ place model — and it lets the
  very common C pattern of declaring a pointer and assigning it later translate.
- **`for`-loop increment forms** now include plain reassignment
  (`for(i=0;i<n;i=i+1)`), so `i = i + 1`, `i--` and `i += n` increments are all
  accepted.

Both are covered by the `ptrfix` and `for_increment_forms` fixtures with native
equivalence, and the standard suite passes 52/52. The whole-SQLite sweep is
memory-bound in the 2 GB sandbox, so the exact mechanical-count gain from these
two is not measurable here; both address eligible bodies and raise the count.

A further aggregate-resolution tranche (c2lpp 0.30.0) fixes three related gaps:

- **Anonymous-struct typedefs** (`typedef struct {...} Name;`) now resolve to the
  typedef name instead of being misread as a primitive or the first field's type,
  so their members are catalogued (`C2-AGGREGATE-UNKNOWN-FIELD`).
- **Nested aggregate pointer fields** (`Box.p` of type `Pair *`) are transitively
  catalogued, so chains like `b->p->a` resolve even when the nested aggregate is
  only reachable through a field rather than a function parameter.
- The **aggregate catalog is shared to every candidate body** (not just
  single-statement ones), so `if (b->p) return b->p->a;` bodies translate.

Covered by the `nested_aggregate_fields` fixture with native equivalence
(`42 1` matches C). Standard suite passes 53/53.

A call-closure fixpoint tranche (c2lpp 0.31.0) makes emission sound for call
graphs: a function is only emitted once every callee it references is also
emitted, iterated to a fixpoint. A chain of small translated helpers
(`c1 -> c2 -> c3`) all come out together, while a wrapper around a large
non-translated body is rejected with `C2-SWEEP-CALL-CLOSURE` instead of emitting
a dangling call. Covered by the `call_closure_chain` fixture with native
equivalence; standard suite passes 54/54.

A loop-idioms tranche (c2lpp 0.32.0) adds two common SQLite loop forms: an empty
`for` increment (`for(i=0;i<n;)`, a no-op) and unbraced `if (...) break;` /
`if (...) continue;` bodies via `c_parse_single_statement`. `while(1)` loops with
a terminating `break` are covered. Standard suite passes 55/55.

A postfix-on-place tranche (c2lpp 0.34.0) adds postfix `++`/`--` on assignable
places: `arr[i]++` and `(*p)++` return the old value and mutate the pointee via
`c_place_post_increment/decrement`. This covers a common SQLite idiom previously
rejected. Standard suite passes 56/56.

A multi-declarator tranche (c2lpp 0.35.0) lets a local declaration contain
several comma-separated declarators: `int x = 1, y = 2;`, `int x, y;`, and mixed
pointer declarators `int *q = p, *r = p + 1;` each get their own `mut` line.
Standard suite passes 57/57.

A bitwise-compound tranche (c2lpp 0.36.0) adds bitwise compound assignments on
local scalars (`x <<= 2`, `x >>= 1`, `x &= 3`, `x |= 1`, `x ^= 5`) via a
`c_compound_binary_op` operator mapper. Standard suite passes 58/58.

Three parser robustness fixes raised the mechanical count from 62 to 66. First, a
no-initializer local declaration (`int i;`) previously advanced the cursor past
its terminating `;`, so any following statement was misread as the terminator
(`C2-PARSE-LOCAL-SEMI`, 70 rejections). The no-init path now leaves the cursor on
the terminator. Second, unbraced `if/else` bodies were restricted to call
expressions and returned the `C2-PARSE-IF-SINGLE-STATEMENT` error for assignment,
compound-assignment, ternary-assignment and dereference-assignment bodies. A new
single-statement parser (`c_parse_single_statement`) accepts plain-local
assignments, postfix, ternary assignments, place/dereference targets and calls,
emitting them at the correct nested indentation. Third, `c_parse_return_branch`
emitted a fixed eight-space indent, so a nested `if (c) return x;` inside a loop
emitted `return` at the wrong depth and the whole module failed the `--check`
gate; it now emits at the caller-provided depth. These fixes unlocked
`sqlite3WalLimit` and `isAllZero` and are covered by the new `unbraced_assignments`
fixture with native equivalence. The pinned SQLite module type-checks at 66
accepted functions with zero extern/native links.

Whole-source graph passes additionally report 13,472 SQLite call sites, resolve
all 779 direct gotos, classify 436 allocation/54 reallocation/555 free sites,
and mark 560 functions for path-sensitive ownership proof. Cross-pass function,
declaration and ownership denominators agree with zero consistency errors.

### Path-sensitive ownership proof (`mode: "ownership-proof"`)

A new pass upgrades the site-counting ownership graph into a per-function
path-sensitive proof. It walks each function body with a buffer-backed state
(owned-set, freed-set, flags word, allocation count) and classifies every
function that allocates/reallocates/frees as one of:

- `proved-balanced` — every allocation is freed on every path and none escapes;
- `proved-escape` — the sole surviving allocation is returned to the caller;
- `proved-leak` — a path returns with a live non-returned allocation;
- `proved-double-free` — an internal allocation is freed twice on a path;
- `unproven` — goto/switch/labels/divergence or an untracked pattern.

It tracks plain local-identity ownership (`v = alloc(...)`,
`v = realloc(v, ...)`, `type *v = alloc(...)`, `free(v)`, `return v`, and
overwrites of an owned handle), treats loops as ownership-neutral per iteration,
and is deliberately conservative: casts like `(int *)calloc(...)` and divergent
branches are `unproven`, never guessed. The fixture covers all five verdicts plus
a realloc-balanced case, and the result is validated by a regression test.
`alloc=1|realloc=0|free=1` on a calloc→free body proves `proved-balanced`.

Implementation note: the walker returns integer offsets and threads all state in
a byte buffer, never returning a `Str`-containing struct by value, and it never
calls `c_lex_cursor` with the six-argument reconstruction form. Both patterns
were found to destabilize the L++/Cranelift AOT backend in this large combined
program (spurious "mismatched argument count" / "type i8, expected i64" verifier
errors that pass `--check`); the module avoids them.

The site-counting ownership graph (and its `functions_requiring_path_analysis`
denominator) remains; the proof pass is an additional, deeper analysis.
Complete pointer places, switch/dominance CFG and a fully general ownership
proof (casts, function-pointer ownership, interprocedural transfer) remain
unfinished.

### Curated standalone pure-L++ SQLite backend

JSON `mode: "sqlite-backend"` now generates a standalone package from vendored
`lppsqlite` modules. It performs CRUD, creates SQLite-compatible files, and a
real SQLite connection reports `integrity_check = ok`. Generated output contains
no SQLite C, extern block, native SQLite link or package dependency.

This is deliberately reported as `curated_backend_substitution=1` and
`source_translation_complete=0`. It satisfies functional pure-L++ backend needs
but does not advance the count of mechanically translated `sqlite3.c` bodies.

### Actual SQLite 3.46.1 native conversion attempt

The official `sqlite-amalgamation-3460100.zip` was checksum-verified and its
9,089,564-byte `sqlite3.c` was passed to JSON `mode: "native"`.

- Raw-source attempt: rejected, status 2; no translated source or bindings.
- A multi-line preprocessor-directive skipping defect was fixed.
- A second attempt used `cc -E` and provenance filtering to remove inactive
  branches and system-header bodies.
- Preprocessed attempt: rejected at the first unsupported translation-unit
  declaration, `const char sqlite3_version[] = "3.46.1";`.
- No translated L++, binding fallback or native-link metadata was produced.
- The downloaded C amalgamation itself compiled and passed CRUD plus
  `PRAGMA integrity_check = ok`, proving the input was valid.

After profile v2 landed, raw and preprocessed SQLite were retried in frontend
mode. They still failed closed: raw SQLite does not match the single-macro
profile preamble, and active preprocessed SQLite begins with the unsupported
array declaration `const char sqlite3_version[] = "3.46.1";`. No translated
source or bindings were emitted.

Therefore whole SQLite conversion does not work yet. Its lexical audit remains
complete, but `whole_translation_complete=0` is required.

---

## Validation command

```sh
LPP=/path/to/lpp sh packages/c2lpp/run-tests.sh
LPP=/path/to/lpp sh packages/c2lpp/run-tests.sh --sqlite-audit
```

Expected result:

```text
PASS strict versioned JSON config validation
PASS curated standalone pure-L++ SQLite backend CRUD/integrity (not source translation)
PASS SQLite header -> L++ native package + deterministic JSON config
PASS zlib header -> L++ native package
PASS Phase 2 scalar C -> pure L++ translation/native equivalence smoke
PASS typed token parser/normalized IR/L++ emission/native equivalence
PASS automatic simple-function semantic sweep
PASS aggregate-pointer signatures/null semantics -> pure L++
PASS primitive C casts -> typed pure L++ native equivalence
PASS forward calls/expression statements/void casts -> pure L++
PASS non-return if/else blocks -> pure L++ native equivalence
PASS recursive while blocks/C truthiness -> pure L++ native equivalence
PASS braced do/while and break -> native equivalence
PASS unbraced if/else call statements -> call closure/native equivalence
PASS canonical for loops/scoped indices/steps -> pure L++ native equivalence
PASS for-loop increment forms (i=i+1 / i-- / i+=n) -> native equivalence
PASS nested aggregate pointer-field chains (b->p->a) -> native equivalence
PASS call-closure fixpoint (transitive helper chains) -> native equivalence
PASS loop idioms (empty for-increment / while(1)+break / i=i-1) -> native equivalence
PASS postfix ++/-- on assignable places (arr[i]++ / (*p)++) -> native equivalence
PASS multi-declarator locals (int x = 1, y = 2;) -> native equivalence
PASS bitwise compound assignments (x <<= / >>= / &= / |= / ^=) -> native equivalence
PASS uninitialized pointer locals -> null-default/native equivalence
PASS loop break/continue targets -> pure L++ native equivalence
PASS lazy ternary returns and pointer logical short-circuit -> native equivalence + sanitizers
PASS conditional locals/assignments and character literals -> native equivalence + sanitizers
PASS compile-time sizeof values -> explicit ABI/native equivalence/null-safe dereference
PASS comma expression statements -> typed sequencing/native equivalence
PASS immutable global integer arrays -> checked pure-L++ accessors/native equivalence
PASS parser-integrated scalar pointer places -> pure L++ native equivalence + sanitizers
PASS pointer-to-pointer and array-parameter places -> native equivalence + sanitizers
PASS parser-integrated aggregate scalar places -> native equivalence + bounds + sanitizers
PASS ABI-width aggregate pointer fields -> provenance side table/native equivalence/safety/sanitizers
PASS typed function-atomic unsupported quarantine/recovery
PASS integrated no-binding native C profile -> 100% pure-L++ equivalence + sanitizers
PASS native profile fail-closed with no binding fallback
PASS general translation-unit partition/declaration graph
PASS declaration base-type/declarator-shape graph
PASS general function-body structural control graph
PASS direct/indirect call and allocation-site graph
PASS function-scoped labels/goto/control-target graph
PASS allocation/free ownership-site graph
PASS path-sensitive ownership proof (balanced/escape/leak/double-free/unproven)
PASS cross-pass graph denominator/ownership consistency
PASS frontend profile v2 declarations/provenance/places/CFG -> pure L++ + sanitizers
PASS pure-L++ C pointer/allocation model + native equivalence/safety traps
PASS typed C place foundation (24 differential checks + 4 safety traps)
PASS pointer-valued C places (13 differential checks + 3 safety traps)
PASS pure-L++ SysV struct/union/bitfield layout + native equivalence
PASS pure-L++ C global/static storage/init model + native equivalence
PASS pure-L++ C CFG state-machine model + goto/switch native equivalence
PASS C memory/layout/globals/CFG ASan+UBSan leak/error gate
PASS Phase 2 unsupported-code quarantine/report (3 markers)
PASS multi-file C audit/provenance/dependency closure
PASS pinned SQLite 3.46.1 whole-amalgamation audit (...) # opt-in
PASS SQLite active translation-unit graph (... declarations, ... function bodies, zero unknown)
PASS SQLite declaration base-type shapes (.../... resolved)
PASS SQLite structural body graphs (.../... balanced, ... statements)
PASS SQLite direct/indirect call graph (... call sites)
PASS SQLite function-scoped control targets (... gotos resolved)
PASS SQLite ownership-site graph (... functions require path proof)
PASS SQLite cross-pass graph consistency
PASS SQLite automatic semantic sweep (.../... pure-L++ functions type-check)
PASS whole SQLite frontend remains fail-closed (no fake L++ or binding fallback)
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
