# c2lpp JSON project configuration

c2lpp reads `c2lpp.json` from its working directory. Conversion settings are
not read from environment variables. The schema is strict: unknown keys,
duplicate keys, trailing commas, trailing data, unsupported value kinds, and
field type mismatches are errors with non-zero process status.

## Schema v1

```json
{
  "schema": "c2lpp-project",
  "schema_version": 1,
  "mode": "frontend",
  "input": "src/library.c",
  "manifest": "",
  "name": "library_native",
  "library": "library",
  "output": "generated/library-native",
  "strict": true,
  "preprocess": false,
  "compiler": "cc",
  "source_version": "1.2.3",
  "source_sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
}
```

| Key | JSON type | Meaning |
|---|---|---|
| `schema` | string | Required literal `c2lpp-project` |
| `schema_version` | integer | Required value `1` |
| `mode` | string | `sqlite-backend`, `graph-check`, `ownership-graph`, `control-graph`, `call-graph`, `sweep`, `body-graph`, `decl-graph`, `tu-graph`, `frontend`, `native`, `bindings`, `audit`, `translate-ir`, or legacy `translate` |
| `input` | string | Header/source path; required except manifest audit |
| `manifest` | string | Multi-file audit manifest; only valid in audit mode |
| `name` | string | Generated L++ identifier; defaults to `clib` |
| `library` | string | Native link name in bindings mode; defaults to `name` |
| `output` | string | Generated directory; defaults to `generated/<name>` |
| `strict` | boolean | Fail if a requested translation/audit is incomplete |
| `preprocess` | boolean | Run `<compiler> -E` in bindings mode |
| `compiler` | string | Shell-safe preprocessor executable atom; defaults to `cc` |
| `source_version` | string | Provenance recorded in audit output |
| `source_sha256` | string | Optional validated 64-hex source digest recorded in audit |

All paths are currently restricted to shell-safe atoms because output directory
creation and optional preprocessing cross the host command boundary. This is an
explicit limitation rather than shell-quoting untrusted text.

## Running

```sh
cp c2lpp.example.json c2lpp.json
./build/c2lpp
```

Each successful operation writes `c2lpp.config.normalized.json` into its output
directory. The normalized form contains defaults and has deterministic key
ordering, making configurations reviewable and cacheable.

## Curated functional SQLite backend

`mode: "sqlite-backend"` vendors the curated pure-L++ SQLite-compatible engine
into the output package. The input path is retained as source identity only; the
backend is not claimed as a source translation. Generated reports distinguish
functional readiness from translator completion.

## General structural graph modes

`mode: "tu-graph"` partitions arbitrary-order top-level C into typedef,
aggregate, global, prototype and function-definition records. It recognizes
function-pointer and variadic declarations, writes exact byte/source spans, and
fails if a top-level declaration cannot be partitioned. It does not translate
function bodies.

On the pinned active preprocessed SQLite 3.46.1 source it records 4,430 external
declarations and 2,528 function bodies with zero unknown top-level records in
about 0.2 seconds on the validation host.

`mode: "decl-graph"` resolves base-type families and records pointer, array,
function-pointer and variadic shape facts for every TU record. On pinned active
SQLite it resolves 4,430/4,430 base-type families, including one target-policy
ABI typedef, with zero unresolved.

`mode: "body-graph"` validates every function body span and inventories
statements, labels, cases, gotos, switches, branches, loops and returns. On
pinned active SQLite it partitions 2,528/2,528 balanced bodies and 45,222
statements. It does not parse typed expression ASTs or resolve CFG edges.

`mode: "sweep"` selects structurally bounded, control-free functions and applies
the typed normalized-IR parser. Only complete accepted functions are emitted;
all others receive stable rejection reasons. The pinned SQLite sweep currently
emits 62 of 2,528 functions (2.45%) as one pure-L++ module that type-checks.

## Strict no-binding frontend mode

`mode: "frontend"` selects profile v2. It generates only `.lpp`, JSON, IR,
report and package metadata files and never falls back to bindings.

Profile v2 integrates a macro with physical/expansion provenance, a forward
struct typedef, callback typedef, nested aggregate and fixed array, const string
and integer-array globals, ordinary and variadic prototypes, checked
`calloc`/`free`, pointer places, a canonical loop and switch/fallthrough/goto
CFG. Declarations without definitions (including the callback and variadic
prototype) are retained in IR but are not emitted as extern bindings.

`mode: "native"` retains the smaller profile v1. Both grammars are fail-closed;
neither is arbitrary C or whole SQLite.

## Multi-file audit

Set `mode` to `audit`, leave `input` empty, and set `manifest` to a project
manifest containing `source=`, `header=`, and `external=` records. Typed
multi-file translation is not implemented yet; only dependency closure audit is
available.

## Non-goals of schema v1

The JSON interface does not make incomplete C semantics complete. Native profile
v1 integrates a narrow pointer/aggregate/global/CFG/allocation grammar, while
general declarations, nested aggregates, callbacks, arbitrary CFGs and whole
SQLite remain translator work. Unknown future settings are rejected rather than
silently ignored.
