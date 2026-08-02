# c2lpp

**Status: Phase 1 is complete; the typed normalized-IR subset works; whole
SQLite and arbitrary-C production readiness are not complete.**

`c2lpp` is a pure-L++ package for conservative C migration. It provides:

1. C-header to L++ native binding/package generation;
2. byte-conserving whole-source and multi-file dependency audit;
3. a typed token → normalized IR → pure-L++ translation path;
4. a strict no-binding native profile integrating declarations, aggregates,
   globals, checked pointers/allocation and CFG lowering.

It never treats an unsupported C semantic as successfully translated.

## Build

```sh
/path/to/lpp src/main.lpp --linker host
```

or:

```sh
LPP=/path/to/lpp sh build.sh
```

## JSON configuration

All conversion settings are read from `c2lpp.json` in the working directory.
The converter does not read `C2LPP_*` environment variables.

```json
{
  "schema": "c2lpp-project",
  "schema_version": 1,
  "mode": "frontend",
  "input": "fixtures/frontend_profile.c",
  "manifest": "",
  "name": "frontend_profile",
  "library": "frontend_profile",
  "output": "generated/frontend-profile",
  "strict": true,
  "preprocess": false,
  "compiler": "cc",
  "source_version": "",
  "source_sha256": ""
}
```

Then run:

```sh
./src/main
```

The parser rejects malformed JSON, unknown or duplicate keys, wrong value
types, unsupported schema versions, invalid field combinations, unsafe path
atoms, and invalid source hashes. A successful operation writes a deterministic
`c2lpp.config.normalized.json` in its output directory.

See [`docs/CONFIG.md`](docs/CONFIG.md) for the complete schema.

## Modes

### `sqlite-backend` — curated standalone pure-L++ functional backend

Copies the repository's tested `lppsqlite` implementation into a generated,
standalone package and emits a small stable adapter API. The generated package
contains no SQLite C source, extern block, native SQLite link or package
dependency. CRUD output is readable by real SQLite and passes
`PRAGMA integrity_check`.

This mode is a curated implementation substitution, **not** mechanical
translation of `sqlite3.c`. Its report always preserves:

```text
functional_backend=1
curated_backend_substitution=1
source_translation_complete=0
```

It provides a production-quality pure-L++ execution backend while the general C
translator continues to expand.

### `tu-graph` — general translation-unit partitioner

This non-emitting frontend accepts arbitrary top-level declaration order and
partitions typedefs, aggregate definitions, globals, prototypes and function
bodies with exact source spans. Function-pointer and variadic declarations are
flagged explicitly.

Pinned active preprocessed SQLite 3.46.1 result:

```text
external_declarations=4430
typedef_declarations=242
aggregate_declarations=209
global_declarations=96
prototype_declarations=1355
function_definitions=2528
function_pointer_declarations=180
variadic_declarations=35
unknown_declarations=0
```

This is structural graph coverage, not implementation conversion.

### `call-graph`, `control-graph`, `ownership-graph`, `ownership-proof`, `graph-check`

These modes build whole-source direct/indirect call records, resolve
function-scoped goto labels, classify allocation/reallocation/free sites, and
cross-check denominators across all structural passes. See
[`docs/GRAPH_ANALYSIS.md`](docs/GRAPH_ANALYSIS.md).
Typed lvalue invariants are documented in [`docs/PLACE_MODEL.md`](docs/PLACE_MODEL.md).

`ownership-proof` additionally runs a per-function path-sensitive ownership
proof that classifies each allocating function as `proved-balanced`,
`proved-escape`, `proved-leak`, `proved-double-free`, or (conservatively)
`unproven`, based on plain local-identity tracking of `alloc`/`realloc`/`free`/
`return` sites. See [`docs/OWNERSHIP_PROOF.md`](docs/OWNERSHIP_PROOF.md).

Pinned SQLite currently yields more than 13,000 call sites, all 779 direct gotos
resolved, and 560 functions requiring path-sensitive ownership analysis. These
are analysis graphs, not emitted SQLite semantics.

### `sweep` — automatic simple-function semantic conversion

Selects balanced control-free function bodies from the general graph and applies
the typed normalized-IR parser. Every accepted function is emitted into one
pure-L++ module; all rejections carry a reason. On pinned SQLite it currently
produces 66/2,528 functions (2.61%) and the resulting module type-checks with no
extern or native link. The sweep resolves 232 scalar/aggregate typedef families,
demand-emits three immutable arrays, and selects ten aggregate type demands under
an explicit memory budget. Scalar and side-table-backed pointer fields retain
checked provenance. Anonymous-struct typedefs (`typedef struct {...} Name;`)
resolve to the typedef name, and nested aggregate pointer fields are
transitively catalogued, so chains like `b->p->a` (including inside `if`
conditions) translate. Conditional lowering now covers lazy returns, local
initializers, ordinary/compound assignments and pointer/null selection. Pointer
logical operators short-circuit, character literals normalize to C integer
values, and `sizeof` emits compile-time primitive, pointer, aggregate and
non-evaluated dereference sizes. Pointer-to-pointer dereference/index/store uses
eight-byte ABI side-table slots, while `T a[]` and `T a[N]` parameters decay to
typed `CPtr` values. Postfix `++`/`--` on assignable places (`arr[i]++`,
`(*p)++`) return the old value and mutate the pointee. Local declarations may
contain multiple comma-separated declarators (`int x = 1, y = 2;`), each getting
its own `mut` line. Multidimensional arrays and pointer depth above two remain
fail-closed. Braced `do/while` loops now preserve first-iteration and bottom-test
semantics; `continue` remains quarantined. Empty statements and unbraced typed
if/else statements are accepted: plain assignments, compound assignments
(arithmetic and bitwise `<<= >>= &= |= ^=`), ternary
assignments, postfix, place/dereference targets and calls. No-initializer local
declarations are parsed correctly: scalars default to zero and pointer locals
(`int *p;`) default to null, so bodies that declare a pointer and assign it later
translate. `for`-loop increments accept `i++`, `i--`, `i += n` and plain
reassignment `i = i + 1` and an empty increment (`for(i=0;i<n;)`); unbraced
`if (...) break;` / `continue;` bodies are supported, covering `while(1)` loops
with a terminating `break`. Emission uses a call-closure fixpoint: a function is
only emitted once every callee it references is also emitted, so transitive
helper chains survive while wrappers around non-translated bodies are rejected
rather than emitting dangling calls. Whole-SQLite do loops remain
outside the memory-bounded selector, which stays at 66 functions, 953 signatures
and 1,017 eligible bodies.

### `decl-graph` — base-type and declarator-shape pass

Consumes the TU graph and resolves primitive, typedef, aggregate and target ABI
type families while counting pointer, array, function-pointer and variadic
shape facts. Pinned active SQLite resolves 4,430/4,430 base-type families with
zero unresolved. Parameter/declarator semantics and ABI layouts are not yet
complete.

### `body-graph` — general structural function graph

Validates balanced function spans and records statements and control constructs
without materializing whole-body token objects. Pinned SQLite result:

```text
functions=2528
bodies_partitioned=2528
bodies_unbalanced=0
statements=45222
labels=240
case_labels=1424
gotos=779
switches=89
ifs=10231
for_loops=1046
while_loops=475
do_loops=89
returns=4054
basic_block_candidates=20955
```

This is not a typed AST and does not resolve label edges.

### `frontend` — expanded strict no-binding profile v2

Profile v2 integrates the requested frontend categories for its tested grammar:

- translation-unit macro and declaration order;
- const string and integer-array globals;
- forward/incomplete struct typedef;
- ordinary, callback and variadic declarations;
- nested aggregate and fixed-array layout;
- macro physical/expansion provenance;
- pointer index/member/address/dereference/arrow places;
- canonical loop and automatic switch/fallthrough/goto CFG;
- allocation call graph, balanced free and no-escape result;
- pure-L++ compatibility-runtime emission.

Callback and variadic prototypes without C definitions are represented in the
normalized graph but never emitted as extern bindings. Generated implementation
code is compiled, executed, compared with native C, and sanitizer-tested.

This remains profile v2, not arbitrary C. SQLite still fails at its general
translation-unit declaration stream.

### `native` — strict no-binding pure-L++ profile v1

This mode integrates the smaller profile v1 end to end:

```text
typedef/aggregate declarations
  -> target layout and integer bitfields
  -> zero/pointer global initializer dependency graph
  -> pointer index/member/address/dereference/arrow places
  -> checked calloc/free ownership and no-escape analysis
  -> canonical loop plus switch/fallthrough/goto CFG
  -> pure-L++ runtime and translated implementation emission
```

The generated package has:

- no `extern "C"` block;
- no native `link` directive;
- no `.c` or `.h` runtime file;
- no fallback to bindings;
- zero unsupported markers for accepted input;
- native C differential and ASan/UBSan/LeakSanitizer gates.

The current grammar is a strict vertical profile represented by
`fixtures/native_profile.c`; arbitrary declarations, nested aggregates,
function-pointer graphs and whole SQLite remain outside it and fail closed.

### `bindings` — older FFI tier, not native conversion

Generates conservative native C ABI declarations, package metadata and an
include dependency report. Example settings:

```json
{
  "schema": "c2lpp-project",
  "schema_version": 1,
  "mode": "bindings",
  "input": "/usr/include/sqlite3.h",
  "name": "sqlite3_system",
  "library": "sqlite3",
  "output": "generated/sqlite3-system",
  "strict": false,
  "preprocess": true,
  "compiler": "cc"
}
```

Implemented boundary:

- optional real C preprocessing;
- comments, multiline declarations and callback-aware splitting;
- numeric macros;
- integer, Float, Bool, strings, handles and callbacks;
- explicit skips for C varargs and unknown by-value ABI.

Bindings still link the native C library. They are not a pure-L++ library
translation.

### `audit`

Scans checked byte buffers, partitions every byte into a lexical class, retains
file/offset/line/column provenance, inventories blocked constructs, and resolves
multi-file include edges.

For a manifest audit, set `input` to an empty string and `manifest` to a file
containing `source=`, `header=`, and `external=` records.

The pinned SQLite 3.46.1 gate audits all 9,089,564 bytes and asserts zero unknown
bytes. Audit output deliberately includes:

```text
whole_translation_complete=0
```

### `translate-ir`

The proof-bearing source translation path is:

```text
checked C bytes
  -> provenance-bearing tokens
  -> scalar C type/symbol resolution
  -> precedence-aware typed expressions and calls
  -> stable normalized C IR
  -> pure-L++ source
```

Current accepted constructs:

- scalar Int/Float/Bool/Void function signatures;
- scalar parameters and local variables;
- unary and binary expressions with precedence;
- assignments, compound assignments and empty statements;
- braced do/while loops without continue;
- unbraced typed if/else call statements;
- typed function calls with arity/argument checks;
- lazy conditional returns, local initializers and assignments;
- pointer-aware short-circuit logical operators;
- C character literals and compile-time `sizeof` values;
- sequenced comma expression statements;
- scalar, pointer-to-pointer and bounded aggregate places;
- decayed one-dimensional array parameters;
- immutable referenced integer-array globals;
- returns;
- function-atomic rejection and parser recovery.

Every accepted fixture is checked, compiled, executed, and compared with native
C. Scalar pointer places, bounded aggregate member/array chains and immutable
integer-array reads are wired to the compatibility runtime. Pointer-valued
aggregate fields, mutable globals, callbacks and unstructured CFG syntax remain
rejected rather than guessed.

## Pure-L++ C compatibility foundations

The package now contains separately tested lowering targets for the next parser
stages:

- `c_memory.lpp`: allocation identities, subobject bounds, mutability,
  one-past pointer arithmetic, checked integer loads/stores,
  `malloc`/`calloc`/`realloc`/`free`, `memcpy`/`memmove`/`memset`, and C strings;
- `c_place.lpp`: flattened provenance-bearing integer/bitfield lvalues plus
  pointer-valued slots, indexing, nested fields, address/dereference, compound
  update, copy/swap, stale-target validation and traps;
- `c_layout.lpp`: target-explicit SysV x86-64 struct/union layout and integer
  bitfield extraction/insertion using raw explicitly destroyed metadata tables;
- `c_globals.lpp`: zero-initialized hidden global/static storage and ordered
  initialization states;
- `c_cfg.lpp`: checked basic-block state machine supporting branch, switch,
  fallthrough, goto-style jumps and returns.

Each successful foundation has native C output-equivalence tests. Memory failure
tests cover out-of-bounds, use-after-free, double-free, interior-free and
readonly writes. All four successful foundations pass ASan+UBSan with
LeakSanitizer enabled on the validated host.

These modules remain **lowering foundations**. The general parser integrates
scalar places, a demand-bounded SysV aggregate subset and side-table-backed data
pointer fields. Function pointers, mutable globals and goto syntax are not yet
parsed.

### `translate`

Legacy compatibility mode for the earlier line-based scalar fixture. New
translator development belongs in `translate-ir`.

## Generated typed package

```text
generated/typed-scalar/
  c2lpp.config.normalized.json
  c2lpp.normalized-ir.txt
  c2lpp.translation-report.txt
  translated.lpp
  src/translated.lpp
  lpp.toml
  README.md
```

A strict typed translation is usable only when:

```text
functions_rejected=0
```

## Test

Standard gates:

```sh
LPP=/path/to/lpp sh tests/run.sh
```

Pinned full SQLite audit as an explicit test option:

```sh
LPP=/path/to/lpp sh tests/run.sh --sqlite-audit
```

The suite validates JSON schema failures, deterministic normalization, SQLite
and zlib binding generation, native library smoke calls, legacy equivalence,
typed normalized-IR equivalence, function-atomic rejection/recovery, multi-file
dependency closure, and the optional pinned whole-amalgamation scan.

## Production boundary

This package is not yet a universal C translator. Production-ready whole SQLite
requires all of the following before completion can be claimed:

- typedef/declaration graph and C integer promotions;
- structs, unions, nested aggregates, bitfields and ABI layout;
- lvalues, pointers, provenance, pointer arithmetic and interior references;
- checked C strings and mutable buffers;
- globals and static initialization;
- function pointers, callbacks and varargs;
- full CFG with labels, goto, switch and fallthrough;
- allocation/reallocation/free ownership policy;
- atomics, threading, setjmp/longjmp and OS/VFS boundaries;
- no unsupported markers;
- no SQLite C linkage;
- real pure-L++ CRUD and SQLite-compatible files;
- differential SQL tests and sanitizer gates.

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md),
[`docs/ROADMAP.md`](docs/ROADMAP.md), and [`HANDOFF.md`](HANDOFF.md).
