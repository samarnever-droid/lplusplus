# c2lpp

**Status: Phase 1 is complete; Phase 2 has started with a working audited
scalar C-to-pure-L++ translator.**

`c2lpp` is a pure-L++ package that generates safe, reviewable L++ native C-ABI
packages from C headers. Its first fixture is SQLite; zlib is also included.

It removes the repetitive work of manually writing hundreds of `extern "C"`
declarations, constants, pointer handles, callbacks, package metadata, native
link names, and dependency reports.

## Tier-1 scope

The first usable tier is a **binding and package generator**:

```text
C header
  -> optional C preprocessing
  -> pure-L++ comment/declaration parser
  -> conservative ABI type policy
  -> generated L++ extern bindings
  -> lpp.toml + dependency report + review diagnostics
  -> native C library linked by L++
```

Phase 2 currently translates an audited subset: scalar functions, typed
parameters and locals, fixed local scalar arrays, checked list-backed indexing
and assignment, canonical ascending C `for` loops, arithmetic, `if`/`else`,
`while`, increment/decrement, calls, and returns. Pointer parameters, unions and
unstructured control flow remain explicit diagnostic comments; they are never
silently presented as safe translations.

## Build

```sh
/path/to/lpp src/main.lpp --linker host
```

This creates `src/main` (or `src/main.exe`).

## Generate SQLite bindings

Linux/macOS:

```sh
C2LPP_HEADER=fixtures/sqlite3_api.h \
C2LPP_NAME=sqlite3 \
C2LPP_LIB=sqlite3 \
C2LPP_OUT=generated/sqlite3 \
C2LPP_CPP=1 \
./src/main
```

Windows PowerShell:

```powershell
$env:C2LPP_HEADER = "fixtures/sqlite3_api.h"
$env:C2LPP_NAME = "sqlite3"
$env:C2LPP_LIB = "sqlite3"
$env:C2LPP_OUT = "generated/sqlite3"
$env:C2LPP_CPP = "0"
.\src\main.exe
```

Generated files:

```text
generated/sqlite3/
  bindings.lpp
  src/bindings.lpp
  lpp.toml
  README.md
  c2lpp.dependencies.txt
```

## Generate zlib bindings

```sh
C2LPP_HEADER=fixtures/zlib_api.h \
C2LPP_NAME=zlib \
C2LPP_LIB=z \
C2LPP_OUT=generated/zlib \
./src/main
```

## Phase 2: translate the audited scalar C subset

```sh
C2LPP_MODE=translate \
C2LPP_SOURCE=fixtures/scalar_algorithms.c \
C2LPP_NAME=scalar_algorithms \
C2LPP_OUT=generated/scalar \
./src/main
```

Output:

```text
generated/scalar/src/translated.lpp
```

The included fixture translates `while` and canonical C `for` summation,
`absolute_value`, `clamp`, and a fixed-array mutation. The translated L++ output
is compared byte-for-byte with a native C reference executable.

Every translation writes `c2lpp.translation-report.txt`. Set
`C2LPP_STRICT=1` to report a failed strict conversion whenever unsupported
construct markers remain.

## Environment contract

| Variable | Meaning | Default |
|---|---|---|
| `C2LPP_MODE` | `bindings` or experimental `translate` | `bindings` |
| `C2LPP_HEADER` | Input C header | required in bindings mode |
| `C2LPP_SOURCE` | Input C source | required in translate mode |
| `C2LPP_STRICT` | Report strict failure if unsupported markers remain | disabled |
| `C2LPP_NAME` | Generated package name | `clib` |
| `C2LPP_LIB` | Native linker library name | package name |
| `C2LPP_OUT` | Output package directory | `generated/<name>` |
| `C2LPP_CPP` | Run `cc -E` with source-marker filtering before declaration parsing | enabled; set `0` to disable |
| `C2LPP_CC` | C preprocessor executable | `cc` |

Paths are restricted to shell-safe atoms in tier 1. This intentionally rejects
spaces and shell metacharacters instead of constructing an unsafe command.

## C-to-L++ ABI policy

| C form | Tier-1 L++ representation |
|---|---|
| signed/unsigned integers, sizes, enums | `Int` |
| `float`, `double` | `Float` |
| `_Bool`, `bool` | `Bool` |
| `const char *` input / character-pointer return | `Str` |
| mutable pointers, opaque handles, pointer-to-pointer | `Int` |
| function pointers and known callback typedefs | `Int` |
| `void` return | `Void` |
| variadic functions | skipped with diagnostic comment |
| unknown by-value structs/unions | skipped with diagnostic comment |

The conservative skip behavior is essential: generating a convenient but wrong
ABI is more dangerous than requiring a manual policy override.

## Test

```sh
LPP=/path/to/lpp sh tests/run.sh
```

The test builds c2lpp in L++, generates SQLite and zlib packages, verifies key
constants/signatures/dependencies, translates scalar C into pure L++, executes
that translation, and—when system headers are installed—generates and natively
calls the real SQLite and zlib libraries.

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) and
[`docs/ROADMAP.md`](docs/ROADMAP.md).
