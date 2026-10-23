# L++ language and toolchain specification

- **Status:** source-grounded description of the implementation in this working tree, not a promise that every construct works on every backend.
- **Audit date:** 2026-10-23. The compact capability inventory in `documentation/CURRENT_CAPABILITIES.md` was last reviewed 2026-10-20.
- **Scope:** the L++ source language, compiler pipeline, runtime/ABI, diagnostics, developer tools, Keel package manager, backend targets, and known limitations.

This document describes what the current implementation parses, type-checks, lowers, and supports on the named backend. It is deliberately not a claim of language-version stability, complete cross-backend parity, data-race freedom, or production readiness for experimental features. A construct accepted by the lexer/parser is not necessarily accepted by type checking or by every code generator. Where support differs, this specification says so.

The generated builtin ABI inventory is incorporated by reference: [`docs/reference/BUILTINS_GENERATED.md`](docs/reference/BUILTINS_GENERATED.md), generated from [`abi/builtins.toml`](abi/builtins.toml). Its target labels are `legacy`; it describes registered names and ABI signatures, **not** a promise that each entry works in the current compiler or on every backend.

## Contents

1. [Compiler model and support language](#1-compiler-model-and-support-language)
2. [Source text, tokens, and layout](#2-source-text-tokens-and-layout)
3. [Declarations, imports, and types](#3-declarations-imports-and-types)
4. [Statements and expressions](#4-statements-and-expressions)
5. [Type system and runtime semantics](#5-type-system-and-runtime-semantics)
6. [Runtime, memory, errors, and concurrency](#6-runtime-memory-errors-and-concurrency)
7. [Modules, packages, and C interoperability](#7-modules-packages-and-c-interoperability)
8. [Compiler pipeline, targets, and backends](#8-compiler-pipeline-targets-and-backends)
9. [Command-line interface](#9-command-line-interface)
10. [Keel package and workspace manager](#10-keel-package-and-workspace-manager)
11. [Developer tools](#11-developer-tools)
12. [Diagnostics and error-code catalogue](#12-diagnostics-and-error-code-catalogue)
13. [Current limitations and validation status](#13-current-limitations-and-validation-status)
14. [Implementation references](#14-implementation-references)

## 1. Compiler model and support language

### 1.1 Pipeline

The current `lpp` executable routes through the rewrite compiler pipeline. Its main stages are:

```text
entry .lpp file
  -> discover and parse imported modules
  -> build HIR and resolve names
  -> infer/check types and collect aggregate, trait, place, and builtin facts
  -> build and verify MIR
  -> run MIR optimization when requested
  -> compute and independently verify ownership proofs
  -> lower MIR to the selected backend
  -> optionally link with the L++ runtime and external libraries
```

The front end preserves source spans and reports lexer/parser/name/lowering diagnostics through the structured diagnostic renderer. Later compiler stages can report typed `E4xxx`/`E5xxx` failures; type-stage errors are currently formatted as `type stage: ...` and do not use the same stable diagnostic-code scheme.

### 1.2 What “supported” means here

- **Parsed** means the lexer/parser accepts the source form.
- **Type-checked/lowered** means the current semantic pipeline can construct typed MIR for it.
- **Backend-supported** means the selected backend can emit it. A backend rejects unsupported MIR with an error; it should not be assumed to produce a partial object.
- **End-to-end verified** means a build/run integration path has actually been exercised. This is a stronger claim than the presence of implementation code or an ABI entry.

The three backends have different surfaces. Consult [§8](#8-compiler-pipeline-targets-and-backends) and [`documentation/CURRENT_CAPABILITIES.md`](documentation/CURRENT_CAPABILITIES.md) before relying on a feature outside Cranelift/native.

### 1.3 Engine selection

`LPP_ENGINE` accepts `rewrite` or `legacy` for compatibility, but the current `src/main.rs` starts `RewriteEngine` in either case. `legacy` prints a retirement notice; it does **not** restore the old compiler. Any other or non-Unicode value exits with status 2. Some old comments/help text still imply a selectable legacy engine; the current entry point is authoritative.

## 2. Source text, tokens, and layout

### 2.1 Files and indentation

- Source files are UTF-8 text. A leading UTF-8 BOM is accepted. Source spans are half-open UTF-8 byte ranges limited to less than 4 GiB per file; rendered columns count Unicode scalar values.
- Blocks are introduced by a trailing `:` and an indented block. Indentation uses spaces. A tab used for indentation is an error; a tab elsewhere in source is also rejected.
- Blank lines and comment-only lines do not create indentation levels. Newlines inside parentheses or square brackets are soft line breaks, so argument and list expressions can span lines.
- A block header cannot have an inline statement after its colon. Top-level executable statements are not allowed: a module contains declarations, attributes, and imports.
- Parentheses and square brackets are recognized delimiters. Curly braces are not general-purpose source delimiters; braces occur only inside formatted-string tokens.

### 2.2 Comments and identifiers

Both `#` and `//` begin a line comment outside a string or character literal. There is no block-comment syntax. Because `//` begins a comment, it is not an integer floor-division operator.

An identifier starts with a Unicode alphabetic character or `_`; later characters may also be Unicode alphanumeric characters. Formatted-string interpolation is narrower: it accepts only one ASCII identifier (`[A-Za-z_][A-Za-z0-9_]*`).

Reserved declaration/control keywords include `as`, `async`, `await`, `break`, `const`, `continue`, `def`, `elif`, `else`, `enum`, `extern`, `fn`, `for`, `from`, `if`, `impl`, `import`, `in`, `match`, `mut`, `pub`, `return`, `spawn`, `struct`, `trait`, `type`, and `while`. `true` and `false` are boolean literals. `and`, `or`, and `not` are operators rather than declaration keywords.

### 2.3 Numeric and literal forms

- Integers are signed 64-bit literals. Decimal, `0x`/`0X` hexadecimal, and `0b`/`0B` binary forms are recognized; underscores may be used as digit separators. Integer literals outside the `i64` range are rejected.
- Floats are decimal digits with a decimal point followed by digits and are stored as `f64`. Exponent notation and numeric suffixes are not part of the current lexer grammar.
- Ordinary strings use double quotes; triple-double-quoted strings are recognized. Character literals use single quotes and must contain exactly one character (or one supported escape).
- Supported escapes are `\n`, `\r`, `\t`, `\0`, `\\`, `\"`, `\'`, and `\xNN` (two hexadecimal digits). Unknown or unterminated escapes are errors.
- Formatted strings use `f"..."`. Each unescaped `{name}` inserts one simple ASCII identifier; `\{` and `\}` escape literal braces. Interpolations are not arbitrary expressions. The current implementation desugars the string into concatenation. Do not rely on triple-quoted formatted strings.

### 2.4 Operators and punctuation

The source lexer recognizes:

- Assignment/binding punctuation: `:=`, `=`, `+=`, `-=`, `*=`, `/=`, `%=`.
- Comparisons: `==`, `!=`, `<`, `>`, `<=`, `>=`.
- Logical operators: `and`/`&&`, `or`/`||`, `not`/`!`.
- Arithmetic: `+`, `-`, `*`, `/`, `%`.
- Bitwise/shift: `&`, `|`, `^`, `<<`, `>>`.
- Postfix/structural forms: calls `(...)`, indexing `[...]`, field access `.name`, numeric tuple projection `.0`, async wait `.await`, propagation `?`, and generic-call forms described below.

There is no `**` exponentiation operator. `=` is not equality; equality is `==`. `:=` introduces an inferred local binding.

The current expression parser uses these precedence groups (highest to lowest; operators in one group associate left-to-right):

| Precedence | Forms |
|---|---|
| Postfix | call, field/tuple projection, index, `.await`, `?`, explicit generic call |
| Unary | `-`, `not`, `!` |
| 4 | `*`, `/`, `%` |
| 3 | `+`, `-`, `&`, `|`, `^`, `<<`, `>>` |
| 2 | `==`, `!=`, `<`, `>`, `<=`, `>=` |
| 1 | `and`/`&&`, `or`/`||` (same precedence) |

The grouping is an implementation fact, including the shared precedence of `and` and `or`, and of arithmetic-additive/bitwise/shift operators; it should not be silently “corrected” to another language’s conventional precedence.

## 3. Declarations, imports, and types

### 3.1 Functions

```lpp
def add(left: Int, right: Int = 1) -> Int:
    return left + right

async def load_name() -> Str:
    return "Indore"
```

Parameters are named and typed in ordinary function declarations. A return annotation uses `->`; omission means `Void`. Default parameter values use `=` and are supported for ordinary functions, but are not supported by external C declarations. `async def` denotes an asynchronous function whose callable result is `Task[body-result]`.

Generic parameters are written in square brackets and may carry a trait bound:

```lpp
def identity[T](value: T) -> T:
    return value

def show[T: Display](value: T) -> Str:
    return value.to_str()
```

The implementation also desugars a parameter annotated with a bare trait name into an implicit trait-bounded generic in the applicable function forms. Generic functions/types are monomorphized. Explicit generic calls are supported through `f::<T>(...)`; the parser also recognizes bracket-style generic calls where they are unambiguous, e.g. `f[T](...)`.

A final typed rest parameter has the form `...items: T`. It is experimental: it must be last, cannot have a default, and the function body sees the rest parameter as `List[T]`. External C variadic declarations/calls are not supported.

### 3.2 Structs, enums, traits, and implementations

```lpp
struct Point:
    x: Float
    y: Float = 0.0

enum Result:
    Ok(value: Int)
    Err(code: Int)

trait Display:
    def to_str(self) -> Str

impl Display for Point:
    def to_str(self) -> Str:
        return "point"
```

- Struct fields require a type and may have a default value. Structs may be generic.
- Enum variants may be nullary or have named, typed payload fields. Variants are constructed and matched as nominal enum values.
- Traits declare signature-only methods. `impl Trait for Type:` implements a trait; `impl Type:` is an inherent implementation. Trait/static and dynamic dispatch exist in the current semantic surface, subject to backend support.
- An implementation’s type parameters may be used by its target/trait and methods. `self` in method signatures is recognized; trait methods use the trait’s `Self` type.
- Constructor calls use the nominal type or variant and positional field values. Field access uses `.name`.

### 3.3 Constants, aliases, attributes, and visibility

```lpp
const ANSWER = 42
type Count = Int

@inline
pub def answer() -> Int:
    return ANSWER
```

Constants and type aliases use `=`. The current type-alias syntax is not generic. Attributes are recorded as simple `@identifier` metadata; the parser does not provide a general attribute argument grammar. `pub` is parsed and carried through HIR, but the current selective-import resolution path does not enforce a public/private access check. Do not use `pub` as a security boundary.

### 3.4 Imports and modules

```lpp
import math.vector
import math.vector as vectors
from math.vector import dot, cross
```

A dotted module path maps to a `.lpp` file (`math.vector` → `math/vector.lpp`). Resolution searches the importing file’s directory and the current package source root first, then declared dependency roots. More than one dependency match is an ambiguity error. Keel stages package sources under `.lpp_packages/` for the compiler resolver.

The single-file CLI compiles one entry file and discovers its imports; passing multiple input files to the compiler CLI is rejected. Module import cycles are allowed by the current compiler entry configuration. Name resolution currently uses a legacy flat global namespace: duplicate global definitions in different modules can be rejected even when a namespaced design might allow them. Explicit imports still have their own binding-conflict checks.

### 3.5 Type syntax

The source type grammar includes named types, applied types `Name[T, U]`, and tuple types `(T, U, ...)`. Tuple types and tuple values have arity 2–4. A parenthesized single expression is grouping, not a one-element tuple; empty tuple expressions are rejected.

The main built-in types are:

| Type | Meaning in the current type/runtime model |
|---|---|
| `Void` | No result value. |
| `Bool` | Boolean value. |
| `Int` | Signed 64-bit integer. |
| `Float` | IEEE-754 64-bit float. |
| `Str`, `String` | Same managed string type. |
| `Char` | Character/code-point type; `print` currently routes it through integer printing. |
| `U8`, `U16`, `U32`; `u8`, `u16`, `u32` | Unsigned fixed-width scalar types and lowercase aliases. |
| `I8`, `I16`, `I32`; `i8`, `i16`, `i32` | Signed fixed-width scalar types and lowercase aliases. |
| `List[T]` | Managed homogeneous list. |
| `Map[K, V]` | Map type; available operations/key representations are backend/runtime subsets. |
| `Slice[T]` | Borrowed list slice/view. |
| `StrSlice` | Borrowed string slice/view. |
| `Task[T]` | Async task/result handle. |
| `(T, U, ...)` | Fixed tuple, arity 2–4. |
| `VectorI64x2` | Experimental two-lane integer vector; operations/backend coverage are limited. |

Source-defined nominal types, generic parameters, and inferred function/closure values are also supported. Function types exist in the semantic model, but there is no general standalone function-type annotation grammar described here. Empty list literals are permitted when context supplies an element type; otherwise inference may be unable to determine it.

## 4. Statements and expressions

### 4.1 Local bindings and assignment

```lpp
name := "Ada"                 # immutable inferred binding
mut total: Int = 0            # mutable typed binding
(total, count) := (12, 3)     # tuple destructuring, 2–4 names

total += count
```

`name := value` introduces an immutable local. `mut name := value` introduces a mutable local. An annotation may be written before either `:=` or `=`. A plain `name = value` is reassignment, not a new inferred binding. Compound assignments are desugared to an operation followed by assignment. Assignment targets are locals or supported field/index places; assignment through an immutable root is rejected. Tuple destructuring is limited to arity 2–4.

### 4.2 Control flow

```lpp
if condition:
    work()
elif other_condition:
    other_work()
else:
    fallback()

while condition:
    step()

for item in items:
    print(item)

match result:
    Ok(value):
        print(value)
    Err(code):
        print(code)
    _:
        print("other")
```

Statements include `return [expr]`, `break`, and `continue`. `if`/`elif`/`else`, `while`, `for`, and enum `match` use indented blocks. `match` patterns are a wildcard `_`, an enum variant name, or a variant qualified by its enum name; payload bindings are positional names in parentheses. General destructuring patterns, match guards, and arbitrary pattern expressions are not part of the current grammar. The type checker records covered/reachable enum arms; use `_` when a fallback is required rather than assuming a language-wide exhaustiveness guarantee.

`for name in list_value` iterates a `List[T]`. A special unresolved `range` call is recognized only in a `for` iterable and lowered to an integer counting loop:

- `range(end)` starts at 0, step 1.
- `range(start, end)` uses step 1.
- `range(start, end, step)` uses the supplied step.
- The upper bound is exclusive (`counter < end`); bounds and step are evaluated once. A negative step does not provide the usual descending-range semantics in the current lowering.
- `range` is not a general first-class runtime function. The special form accepts 1–3 arguments.

### 4.3 Expressions, closures, tasks, and propagation

Expressions include literals, names, unary/binary operations, calls, list/tuple values, field access, indexing, closures, `spawn`, `.await`, and postfix `?`.

- `fn(x: Int) -> Int: x + 1` is an expression-bodied closure with an implicit return. A closure may also use an indented statement body after `fn(...):`. Closure parameter annotations may be omitted and inferred.
- `spawn expression` schedules a spawned closure/callback through the host thread path where available. It is separate from `async`/`Task` and is not supported by freestanding minimal runtimes.
- Await is postfix: `task.await` or `make_task().await`; it is intended for `async def` bodies.
- `value?` propagates an enum residual from a function. The operand’s enum must have its success variant first with exactly one payload. The enclosing function must return the same enum definition, and residual payload types must be compatible. This is a narrow enum convention, not a built-in exception system or a general `Result` constraint.
- Formatted strings are `f"...{name}..."`; an interpolation is one identifier, not an arbitrary expression. The current checker explicitly supports string concatenation/stringification of `Int`, `Float`, and `Bool` interpolation values. Do not assume all types, including `Char`, can be interpolated.
- Tuple access accepts a constant numeric projection such as `pair.0`; tuple indices must be static and in range. Ordinary list/string indexing uses `[...]`.

## 5. Type system and runtime semantics

### 5.1 Inference and checking

The current compiler performs type inference/checking over HIR, records typed facts for MIR construction, and monomorphizes generic instances. It checks function/call arity and types, nominal constructors/fields, enum variants/payload binding counts, assignments, tuple arity/indexing, generic substitutions, trait implementation coherence/selection, and the experimental `?`, slices, tasks, and spawn rules.

The implementation is bounded: type inference has work/depth budgets; trait solving and generic instance collection have configurable/default limits. Hitting a work/depth/instance limit is an error, not an invitation to assume an unconstrained type. This is not a formal specification of all inference/generalization behavior.

Current type-stage error variants include: mismatch and arity mismatch; occurs-check/infinite-type; inference/interner capacity exhaustion; type work/depth limits; invalid bound variables and type-alias cycles; unknown field/variant and invalid aggregate; invalid/immutable assignment target; non-static or out-of-range tuple index and tuple-element assignment; invalid match subject/pattern or payload binding arity; invalid `?` carrier/success arity/return/residual; mutation of a spawned closure capture; blocking builtin in an async call graph; task capture by a closure; and borrowed-slice escape. These are formatted as `type stage: ...`; they currently do not carry the stable diagnostic-code family used by the lexer/parser.

### 5.2 Numeric, boolean, and string operations

- `Int` arithmetic is signed 64-bit wrapping arithmetic for addition, subtraction, and multiplication. Integer division/remainder by zero is a defined runtime panic; `INT_MIN / -1` wraps to `INT_MIN`, and `INT_MIN % -1` is zero. Shift counts are masked by the integer backend semantics.
- `Float` uses IEEE-754 64-bit arithmetic; division by zero follows IEEE (`inf`/`NaN`), not the integer divide-by-zero panic.
- Logical `and`/`or` use short-circuit control flow. Their parse precedence is the shared lowest operator level shown in §2.4. Comparison and binary operand validity is still constrained by type inference/backend support.
- String `+` is concatenation when the left operand is a string; the current type checker also permits a scalar `Int`, `Float`, or `Bool` on the right via conversion. It does not promise symmetric coercion.
- `Str` is NUL-terminated byte data. The standard runtime’s `str_len`, `char_at`, `str_substr`, string slices, search/compare, and many related operations are byte-oriented; case conversion is ASCII-only in the native runtime. `chr` encodes a valid Unicode scalar and `ord` decodes the first scalar when valid. `Str` is not a binary-safe container; use a buffer type for arbitrary binary data.

### 5.3 Managed values and borrowed views

Strings, escaping aggregates, closures, tuples/tasks, lists, and related managed runtime objects use ARC ownership. The ownership analysis distinguishes frame-resident values, acyclic owned heap values, and cycle-capable shared values. Nonescaping ordinary values may be stack-resident; escaping values use runtime allocation and generated drop logic.

On the Cranelift native path, reference counts are atomic and the runtime includes a trial-deletion collector for unreachable cycles among collectible shared objects. This is a native-path guarantee, not a universal backend guarantee. The WebAssembly runtime reuses released acyclic heap blocks but does not collect unreachable cycles; cyclic WASM values can therefore leak. LLVM managed-value/cycle-collector parity is incomplete.

`Slice[T]` and `StrSlice` are first-tier borrowed views with conservative escape checks, runtime bounds/generation validation, and no general source-level lifetime parameters. They may not safely escape their source, be returned/stored/captured, reach an unknown retaining call, or cross `spawn`; use a conversion such as `slice_to_str`/`str_slice_to_str` to make an owned string. Mutable/general lifetime-parameterized slices are not claimed.

## 6. Runtime, memory, errors, and concurrency

### 6.1 Runtime I/O

The following describes the hosted Rust runtime’s current native behavior. Freestanding and WASI backends have their own formatting implementations, so byte-for-byte output should not be assumed identical across all targets.

| Operation | Hosted native behavior |
|---|---|
| `print_int(x)` | Signed decimal followed by newline (`%lld\n`). |
| `print_float(x)` | `%f\n`, normally six digits after the decimal point. |
| `print_bool(x)` | `0\n` for false and `1\n` for true. |
| `print_str(s)` | String plus newline (puts-like). |
| `print(x)` | Type-directed dispatch; `Char` is widened to the integer printer, not printed as a Unicode glyph. |
| `write_str(s)` | String bytes, no trailing newline. |
| `write_int(x)` / `write_uint(x)` | Decimal with no trailing newline. |
| `write_float(x)` | `%g`-style number with no trailing newline. |
| `write_bool(x)` | `true` or `false`, no trailing newline. |
| `write_char(x)` | Emits the low byte of the value, not a general UTF-8 encoding. |
| `write_nl()` | Emits one newline. |
| `eprint_str(s)` / `ewrite_str(s)` | Write to stderr; `eprint_str` adds a newline, `ewrite_str` does not. |
| `input()` | Reads one line, removes LF and a trailing CR, and returns an owned `Str`; EOF/read error returns an empty string. |

Hosted stdout writes flush. String-producing runtime functions return managed strings; null inputs to many string helpers are normalized to empty data according to the helper’s contract. See [`crates/lpp-runtime/src/io.rs`](crates/lpp-runtime/src/io.rs) and [`crates/lpp-runtime/src/string.rs`](crates/lpp-runtime/src/string.rs) for operation-specific behavior.

### 6.2 Panics and recoverable failures

Expected failures should be represented as values (often an enum) and propagated with `?`. Contract violations use the runtime panic path. Hosted panic handling flushes stdout, prints the `L++ RUNTIME PANIC` banner and reason to stderr, and terminates the process with status 101. It does not unwind; destructors on the panic path do not run. Minimal freestanding runtimes may exit without a stderr banner. There is no general `catch`/recovery or exception-unwinding contract.

Runtime panic conditions include supported bounds/handle failures, integer division/remainder by zero, allocation failure, and explicit assertion failure. Exact runtime checks vary by backend. The MIR interpreter’s `E430x` diagnostics are oracle/developer errors, not the hosted runtime’s public panic text.

### 6.3 Concurrency and async

- `spawn` uses host threads on supported hosted paths. The type checker rejects mutation of a mutable captured local in a spawned closure and rejects borrowed slices crossing spawn. Freestanding minimal runtimes have no thread primitive; spawn there is not linkable.
- ARC refcounts are atomic in the current runtime. This is not a claim of general data-race freedom: shared mutable struct fields can still race. Treat data shared between threads as immutable, atomic-only, or lock-protected. The cycle collector’s interactions with concurrent mutation are not fully proven for arbitrary shared cyclic graphs.
- `async def`/`.await` use task-state records and a deterministic, single-thread, run-to-completion executor. There is no I/O reactor, nonblocking socket/file readiness, work stealing, fairness, or general coroutine suspension guarantee. A blocking builtin called transitively from an async function is rejected in the current first tier. Task handles cannot be captured by closures in this tier.
- Panics terminate the process; there is no per-thread panic isolation or join-error recovery guarantee.

See [`documentation/CONCURRENCY.md`](documentation/CONCURRENCY.md), [`documentation/PANIC_AND_ERRORS.md`](documentation/PANIC_AND_ERRORS.md), and [`documentation/CYCLE_BREAKING.md`](documentation/CYCLE_BREAKING.md).

## 7. Modules, packages, and C interoperability

### 7.1 Package/module resolution

The compiler’s module graph translates dotted imports into `.lpp` paths. For a module imported from a source file, it checks the importer directory and current package source root, then dependency source roots. An unambiguous dependency match is required. Keel’s dependency staging makes path/registry package source roots discoverable to the compiler. The compiler currently permits module cycles.

The resolver records `pub`, but the inspected selective-import resolution does not enforce it. The compile pipeline also selects `LegacyFlat` resolution: duplicate top-level names across modules can produce `E3004`. Do not infer a stable namespaced or privacy model from the import syntax alone.

### 7.2 Imported C ABI

The current source tree contains an imported-extern implementation in the MIR/link/codegen path. It records external function metadata (source item, symbol, ABI, optional library, function type, origin), represents external calls by an external-function ID, and propagates distinct library names to the native linker. MIR snapshots use the `mir-v7` version after this representation change.

The supported source form is a signature-only C block:

```lpp
extern "C" link "sqlite3":
    def sqlite3_libversion() -> Str
```

The implemented native Cranelift subset is direct, non-variadic `extern "C"` calls with supported scalar/`Str` signatures. `Void` is allowed only as a result. Current supported external parameter/result types are `Bool`, `Int`, `Float`, `U8`/`I8`, `U16`/`I16`, `U32`/`I32`, and `Str`; `Char`, aggregates, callbacks, generic functions, async functions, defaults, and C variadics are rejected. The declaration is the ABI contract: `Int` is 64-bit, so use a fixed-width L++ type for a narrower C integer. A foreign pointer/handle carried in `Int` remains the caller’s responsibility.

`link "sqlite3"` is a bare linker-library name, not a command-line flag or path. The driver validates names and passes the distinct libraries to the selected native linker. Linking still depends on the platform toolchain/library being installed.

`Str` is **not** an owned raw C pointer. Its machine-level value is a pointer to a NUL-terminated payload, with a 24-byte ARC header immediately before it on the hosted native layout. For an L++ `Str` argument the native external-call lowering passes the payload pointer as a **borrowed** C string for the duration of the call. Foreign code must not mutate it or retain it after returning. A C `char *` returned as `Str` is treated as borrowed readable NUL-terminated data and copied immediately into a new ARC-managed L++ string; the foreign pointer is not adopted or freed. A null string result follows the helper contract and becomes an empty string. These are FFI preconditions, not safety checks the compiler can prove.

The LLVM and WASM backends explicitly reject imported external calls. The L++/SQLite end-to-end smoke has not been established by the current audit: the recorded `Unsupported(UnresolvedName)` failure was not re-run after the current source changes. Symbol/link metadata tests do not prove a SQLite library was linked and executed. See [`documentation/EXTERNAL_C_ABI.md`](documentation/EXTERNAL_C_ABI.md).

## 8. Compiler pipeline, targets, and backends

### 8.1 Backend matrix

| Backend | Declared target(s) | Current support boundary |
|---|---|---|
| Cranelift | `x86_64`, `aarch64`, with the object OS format inherited from the compiler host | Default native backend and broadest rewrite backend. Supports the current native scalar/control-flow/direct-call path and substantial managed-data/runtime surface (strings, structs/enums, lists, tuples, closures, tasks, slices and supported builtins). Direct external C calls are implemented for the subset in §7. SIMD and other builtins remain feature-specific. |
| LLVM | `x86_64` only | Optional Clang-based scalar/CFG/direct-call and builtin subset. Managed structs/lists/tuples, function values, slices, SIMD, and imported C calls are incomplete or typed rejections in the current slice. Windows LLVM runtime execution still needs validation. |
| WASM | `wasm32-wasip1` | Experimental WASI backend. Current source implements scalars, control flow/direct calls, strings, managed aggregates/lists, tuples, dynamic string concatenation, closures/function values, tasks, eager spawn, borrowed slices, deterministic maps, and WASI line input in its tested subset. SIMD, arbitrary host C FFI, and host capabilities unavailable in WASI are rejected. Cycle collection is missing. |

This matrix is a feature boundary, not a claim that every listed feature is equally mature or that every builtin is available on every target. The complete current feature index is [`documentation/CURRENT_CAPABILITIES.md`](documentation/CURRENT_CAPABILITIES.md).

### 8.2 Target selection and cross-compilation

The shared codegen target lattice is exactly `X86_64`, `Aarch64`, and `Wasm32Wasi`. Native triples use the compiler host OS (Linux, Windows, or macOS) with the selected architecture; explicit cross-OS/sysroot support is outside the current v0.1 model. A native cross-architecture object can be emitted/checked, but the single-file `build`/`run` path only links the current host target.

The CLI accepts aliases including `x86_64`, `aarch64`, `aarch64-linux-android`, `wasm32`, `wasm32-wasi`, `wasm32-wasip1`, and `wasm32-unknown-unknown`. These aliases map into the small target lattice: for example, `aarch64-linux-android` maps to the generic AArch64 target and does not supply an Android sysroot/linker, and `wasm32-unknown-unknown` is mapped to the WASI backend rather than a true unknown-unknown target. `--list-targets` includes compatibility labels that are not all independently implemented targets; use the backend target list and actual compile result as the authority.

For WASM, the output is a `.wasm` module and there is no native host link step. `--run` invokes `wasmtime`; if it is absent, the module is written but execution fails with an install/runtime message. A nonzero WASM optimization level invokes the external `wasm-opt` tool; `-O0` leaves the module unprocessed by that tool. The LLVM backend invokes Clang (`LPP_LLVM_CC`, configured LLVM path, or `clang`) to create its object. Cranelift’s native code generator sets its own native optimization behavior; the shared `-O` value should not be read as a universal native backend optimization guarantee.

### 8.3 Native linking

The native executable links against the L++ runtime dynamic library (`liblpp_runtime.so`, `.dylib`, or `.dll`) and any declared external libraries. The rewrite driver supports a direct in-process linker path and a host linker path. Direct linking is platform/architecture/format constrained; the driver may fall back to the host C linker on limitations. `--linker cc` and `--linker host` select the host toolchain. The standalone `lpp-link` binary has a broader explicit ELF/PE/Mach-O interface than the current compiler-driver direct-link path; do not infer that every standalone format is available through `lpp <file> --linker direct`.

The runtime can be located with `LPP_RUNTIME_LIB` (an explicit runtime file) or `LPP_RUNTIME_DIR` (a search directory), otherwise the driver checks its installed toolchain/source-tree locations.

### 8.4 Optimization levels

The direct compiler CLI defaults to `O0`. Accepted level spellings include `0`/`O0`/`none`, `1`/`O1`/`basic`, `2`/`O2`/`speed`, `3`/`O3`/`aggressive`, `s`/`Os`/`size`, and `z`/`Oz`/`min-size`. `O0` skips the MIR optimization pipeline but still performs final MIR verification and ownership proofs. Higher levels select bounded MIR optimization budgets; native backends do not all consume the post-codegen optimization setting in the same way. On WASM, nonzero levels map to `wasm-opt` passes; `Os`/`Oz` use the backend’s size-oriented MIR budgets and are mapped to its available WASM optimization level.

## 9. Command-line interface

### 9.1 Direct source compilation

```sh
lpp app.lpp                         # build a native executable
lpp app.lpp --run                   # build and run; propagate child exit status
lpp app.lpp --check                 # compile to an object, do not link or write it
lpp app.lpp --emit-object -o app.o  # write a native object
lpp app.lpp --backend llvm          # select LLVM
lpp app.lpp --backend wasm          # emit WASI WebAssembly
lpp app.lpp --target aarch64        # select a native architecture target
lpp app.lpp -O2 -o app
```

Supported compiler options include:

- `--backend cranelift|llvm|wasm` (`--llvm` is shorthand for `--backend llvm`).
- `--target <target>`; selecting WASI also selects the WASM backend.
- `-O<level>`, `-O <level>`, or `--opt-level <level>`.
- `-o <path>` / `--output <path>`.
- `--run`, `--check`, `-c`, `--emit-object`, `--emit-obj`, and `--aot`.
- `--list-targets`, `--help`/`-h`, and `--version`/`-v`.
- `--linker direct|cc|host` for native build/run.

Native `--check` runs the compiler through object generation but skips linking and does not write the object. WASM `--check` emits and validates a module in memory. `--emit-object` writes `.o`/`.obj` for native targets and `.wasm` for the WASM backend. A non-host native target cannot be linked/run by the direct source path; use `--check` or `--emit-object`.

The CLI accepts one source input. Multiple positional source files are rejected; imported modules are found through the module graph.

### 9.2 Project commands and dispatch

When a command receives a source-file argument (for example `lpp run app.lpp`), the compiler handles the file. Without a source argument, project/package commands are dispatched to Keel in-process: `new`/`create`, `init`, `add`, `remove`, `build`, `check`, `run`, `test`, `clean`, `metadata`, `list`, `workspace`, `version`, `install`, `fetch`, `search`, `publish`, `update`, `tree`, `outdated`, `why`, `verify`, and `cache`.

The rewrite compiler currently rejects `--dump-*`, `--checkall`, and `--fix`; these are not available in the current direct-source rewrite path. Old `Usage.md`/help claims for these options should not be treated as current support.

### 9.3 Configuration and utility routes

- `lpp config` prints configuration; `lpp config set backend <cranelift|llvm|wasm>`, `linker <direct|host|auto>`, or `llvm-path <path>` writes `~/.lpp/config.json` (or the platform home equivalent).
- `lpp doctor` reports host/runtime/backend/LLVM/WASM-runtime availability.
- `lpp bench [...]` launches `lpp-bench` beside `lpp`, or `LPP_BENCH_BIN` if set.
- `lpp upgrade` checks the release channel; automatic binary replacement is intentionally not performed. `setup`/`toolchain` routes configure/check external tooling.
- `lpp dev` maps to Keel run. The old bundled Lreact project generator and `lpp login` token flow are retired; `lpp lreact` is a limited compatibility route.
- If `LPP_AOT` or `LPP_AOT_ONLY` is present in the environment, the direct-source path selects object/module emission.
- If `BENCHMARK` is present, the compiler prints a `TIMING_JSON` line. The current rewrite CLI measures total duration, but its displayed per-stage subdivisions are placeholders/equal splits, not trustworthy phase-by-phase profiling data.

`LPP_ENGINE=legacy` is not an escape hatch to the old compiler; see §1.3. The CLI exit status for a normal compile/build error is 1; usage/unsupported command paths commonly return 2. `lpp run` propagates the child’s status, and a child killed by a signal is reported as `128 + signal` on Unix.

## 10. Keel package and workspace manager

Keel is the package/workspace manager embedded behind project commands and also has a standalone `keel` CLI.

### 10.1 Project layout and manifest

`keel new name` creates `Keel.toml` and `src/main.lpp`; `keel init` initializes the current directory without overwriting existing source. A package entry point is `src/main.lpp` (binary) or `src/lib.lpp` (library).

A minimal manifest is:

```toml
[package]
name = "demo"
version = "0.1.0"
edition = "2024"

[dependencies]
```

The current package-name rule is 1–64 lowercase ASCII letters, digits, or hyphens, beginning with an alphanumeric character. Versions and dependency requirements use semantic-version syntax. The manifest model includes dependencies, features, optional target support, and workspaces. Dependencies may be a version string or a detail table (version, optional/features, or a local path); **git dependencies are explicitly rejected by the current manifest validator**. Path dependencies must resolve as workspace members within the workspace.

A workspace may have `[workspace] members = [...]`; a member pattern may contain at most one `*` and it spans one path segment. `**` and multiple stars are unsupported. Each member has its own `Keel.toml`. A virtual workspace root can omit `[package]`.

Keel’s user-facing package/workspace manifest is `Keel.toml`. Keel also writes internal/staged `lpp.toml` files under `.lpp_packages/` so the compiler’s module resolver can import staged dependency source. These are different files with different roles.

### 10.2 Commands and behavior

The Keel command surface includes:

| Command | Purpose |
|---|---|
| `new`/`create`, `init` | Create or initialize a project. |
| `add`, `remove` | Edit dependencies. `add` accepts a package name or `name@version`; a `--path` CLI option is not part of the current clap surface. A path dependency can be declared in the manifest. |
| `build`, `check`, `run` | Build workspace/member artifacts, check a selected package entry, or build and run it. |
| `test` | Run each `tests/*.lpp` as a standalone compile-and-run test; no test files is a successful “no tests found” result. |
| `clean` | Remove workspace build outputs. |
| `metadata`, `list`, `workspace members|graph` | Inspect packages, dependencies, and workspace membership. |
| `workspace build [package]`, `workspace test [package]` | Build/test a workspace or selected member and its path dependencies. |
| `version [set|bump]` | Read or update package SemVer. |
| `install`, `fetch`, `search`, `publish`, `update` | Resolve/fetch dependencies or interact with a Git-backed registry. |
| `tree`, `outdated`, `why`, `verify` | Inspect the resolved dependency graph, versions, dependency chains, or locked-artifact integrity. |
| `cache stats|clean|path` | Inspect/clean the global cache. |

Workspace builds topologically order path-dependency layers and can build members in a layer concurrently (`KEEL_JOBS` controls parallelism). Path dependencies are staged under `.lpp_packages/`; registry artifacts are checked against lockfile checksums/tree hashes. `Keel.lock` supports lock formats 1 and 2; registry artifacts carry SHA-256 checksums. `keel check` is described as a check operation but invokes the compiler’s direct-source `--check` route, which currently generates/validates backend object code rather than being a semantic-only front-end pass.

The durable cache is under `$XDG_CACHE_HOME/keel` when set, otherwise `~/.cache/keel` (Windows uses `LOCALAPPDATA/Keel`). The global `--cache-backend` option currently selects the `auto`/`memory` hot-cache implementation; durable content blobs and lockfiles remain the source of truth.

`--registry <git-url>` or `KEEL_REGISTRY` selects a registry. Registry commands that have no explicit registry may use the built-in default Git registry; commands requiring an explicitly configured registry return a clear error when none is available. Git transport credentials are used; Keel does not store a plaintext publisher token.

### 10.3 Keel package-manager error codes

Package-manager errors use `E6000`–`E6026`; see the complete mapping in §12. The Rust package manager is the current production-default route. A pure-L++ package-manager experiment is not the default compiler/package command path.

## 11. Developer tools

### 11.1 Language server

`lpp-lsp` is a stdio JSON-RPC/LSP process using `Content-Length` framing. It advertises full text synchronization, completion, hover, definition, document symbols, formatting, and full semantic tokens. Its implemented scope is substantially more limited:

- It parses documents and publishes **parser-only** diagnostics on open/change/save; it does not run the full compiler type checker or package resolver.
- Completion is hard-coded plus same-document function/struct/enum names. It is not a workspace/import-aware semantic completion engine.
- Hover is line-text heuristic; definition search is heuristic and same-document only; document-symbol ranges are approximations; formatting is a simple indentation heuristic, not a syntax-aware formatter.
- Semantic tokens are advertised by `initialize`, but the current server has no `textDocument/semanticTokens/full` request handler.
- Some hard-coded completion/hover labels (`CPtr`, `CMemory`, `Buffer`) are legacy placeholders, not proof that those names are built-in source types.

### 11.2 Linker and compiler API

`lpp-link` is a standalone direct-linker/inspection CLI. It accepts ELF, PE, and Mach-O modes, object inputs (and supported archives), `-o`, entry/library/search-path and format options, response files, and `inspect <object-or-archive>`. It is a separate executable; the compiler driver’s direct-link support is narrower as described in §8.3.

The backend API is defined in `lpp-codegen-api`: a backend declares target support and consumes verified MIR plus type/name metadata; unsupported constructs are typed errors. `lpp-common` provides target, optimization, source-map, and diagnostic models. The HIR, type, MIR, passes, ownership, linker, and runtime crates are also developer-facing implementation boundaries, not separate end-user language promises.

### 11.3 `c2lpp` / `lpp-bindgen`

`packages/lpp-bindgen` is an experimental pure-L++ C analysis, binding-generation, audit, and narrow translation project. It is not a general C-to-L++ translator and is not production-complete.

The strict JSON schema v1 (`c2lpp.json`) recognizes modes including `bindings`, `audit`, `sqlite-backend`, `graph-check`, `call-graph`, `control-graph`, `ownership-graph`, `ownership-proof`, `sweep`, `body-graph`, `decl-graph`, `tu-graph`, `frontend`, `native`, `translate-ir`, and legacy `translate`. Unknown keys/value kinds fail closed. The frontend/native profiles and typed sweep accept a limited grammar and report unsupported constructs; multi-file audit is dependency-closure analysis, not general multi-file translation. `sqlite-backend` vendors a curated compatible backend and does not claim translation of the input C source. Paths are restricted to shell-safe atoms; optional preprocessing invokes a configured host compiler. See [`packages/lpp-bindgen/docs/CONFIG.md`](packages/lpp-bindgen/docs/CONFIG.md).

## 12. Diagnostics and error-code catalogue

### 12.1 Diagnostic shape and rendering

A structured diagnostic has a code, severity (`error`, `warning`, `note`, `help`), message, optional primary source span, labels, notes, and help. Codes have the form one uppercase letter followed by four digits. The human renderer prints the code/message and, when source is available, a file/line/Unicode-scalar-column location and source line.

Not every compile failure is a `Diagnostic`. The type checker and graph/link stages may be rendered as stage-prefixed text; runtime panics use the runtime panic banner; Keel uses its own errors. Code families below describe the stable literal codes found in the current implementation, not a guarantee that every failure path emits one.

### 12.2 Lexer and parser

| Code | Current meaning |
|---|---|
| `E1000` | Source file exceeds the 4 GiB span limit. |
| `E1001` | Unexpected/invalid character or non-indentation tab. |
| `E1002` | Tab used for indentation. |
| `E1003` | Inconsistent indentation level. |
| `E1004` | Invalid/unknown/unterminated string or character escape/literal. |
| `E1005` | Numeric literal is invalid or out of range. |
| `E1100` | Empty module/no declarations. |
| `E1101` | Mismatched or unclosed delimiter. |
| `E1102` | Invalid top-level statement/declaration. |
| `E1103` | Invalid or unattached attribute. |
| `E1104` | Missing/invalid block header or indentation. |
| `E1105` | Typed local binding lacks an initializer. |
| `E1106` | Unexpected indentation/dedent. |
| `E1107` | Invalid declaration header/name/parameter-list requirement. |
| `E1110` | Tuple syntax exceeds the current 2–4 element limit. |
| `E1111` | Variadic rest parameter is not final. |
| `E1120` | Malformed module/selective import. |

### 12.3 Name resolution and HIR lowering

| Code | Current meaning |
|---|---|
| `E3001` | Duplicate definition in one module. |
| `E3002` | Selectively imported name does not exist in the module. |
| `E3003` | Import binding conflicts with another name. |
| `E3004` | Duplicate name in the compiler’s legacy flat namespace. |
| `E3099` | HIR resolver/interner/arena capacity failure. |
| `E3100` | Unsupported or malformed HIR source construct/type/expression. |
| `E3101` | Declaration in a function body or invalid signature-only body. |
| `E3102` | Invalid assignment target. |
| `E3103` | Orphaned `elif`/`else`/match arm. |
| `E3104` | Invalid literal or malformed formatted string. |
| `E3105` | Formatted-string interpolation is not one simple identifier. |
| `E3199` | HIR lowering storage/capacity failure. |

### 12.4 MIR construction, verification, passes, interpreter, and ownership

| Code | Current meaning |
|---|---|
| `E4001` | MIR builder capacity failure. |
| `E4002` | Unsupported MIR/source construct; includes invalid/unsupported external ABI/library/signature. |
| `E4003` | Missing type or invalid function type during MIR construction. |
| `E4004` | Invalid assignment target during MIR construction. |
| `E4005` | Missing required return. |
| `E4006` | Inconsistent/missing inferred MIR fact, instance, aggregate, builtin, await, or spawn data. |
| `E4101` | Missing/cross-function MIR reference or invalid/duplicate generic instance data. |
| `E4102` | MIR entry block outside a function. |
| `E4103` | MIR structural/type/arity/operand/store/aggregate/call invariant failure. |
| `E4104` | Read of an uninitialized MIR local. |
| `E4105` | Definite-initialization analysis resource limit. |
| `E4201` | MIR fails verification before an optimization pass. |
| `E4202` | Optimization pass precondition is not established. |
| `E4203` | Optimization pass fails. |
| `E4204` | Optimization pass produces invalid MIR. |
| `E4301` | MIR interpreter invalid program/function/arity/value type. |
| `E4302` | MIR interpreter execution limit. |
| `E4303` | MIR interpreter runtime fault (invalid local/condition/call/projection, unsupported operation/builtin/external, bounds, division, overflow, unreachable, etc.). |
| `E4304` | MIR ARC oracle ownership underflow or unexplained ownership leak. |
| `E4401` | Ownership plan cannot be constructed from MIR. |
| `E4402` | Independent ownership-plan proof does not match the computed plan. |
| `E4403` | Ownership balance proof failure, including use-after-move/missing ownership cell. |

The `E430x` family belongs to the MIR interpreter/oracle; it is not the same as the hosted runtime panic status/text.

### 12.5 Code generation and driver

| Code | Current meaning |
|---|---|
| `E5001` | Backend does not support a MIR construct. |
| `E5002` | Backend does not support the requested target. |
| `E5003` | Builtin is unrepresentable/unsupported by that backend. |
| `E5004` | Backend IR verification failure. |
| `E5005` | Object/module emission failure. |
| `E5006` | Backend ABI descriptor mismatch. |
| `E9000` | Driver request/context construction failure. |

Target, graph, filesystem, I/O, and link errors are also surfaced as stage-prefixed compile errors; they do not all have a stable `E` code.

### 12.6 Keel package-manager codes

| Code | Current meaning |
|---|---|
| `E6000` | Invalid package name. |
| `E6001` | Invalid content address. |
| `E6002` | Invalid semantic version. |
| `E6003` | Cache/blob-store I/O error. |
| `E6004` | Blob not found. |
| `E6005` | Value exceeds cache size limit. |
| `E6006` | Manifest parse/validation failure. |
| `E6007` | Package not found in registry. |
| `E6008` | Version missing or yanked. |
| `E6009` | Checksum mismatch. |
| `E6010` | Malformed registry index. |
| `E6011` | Git invocation error. |
| `E6012` | Git command failed. |
| `E6013` | Dependency-version conflict. |
| `E6014` | No package version satisfies a requirement. |
| `E6015` | `Keel.lock` parse/validation failure. |
| `E6016` | Workspace path-dependency cycle. |
| `E6017` | Workspace member lacks a package manifest. |
| `E6018` | Duplicate workspace member name. |
| `E6019` | Path dependency is not a workspace member. |
| `E6020` | Unsupported workspace-member glob. |
| `E6021` | Fingerprint-store error. |
| `E6022` | Locked version is no longer available. |
| `E6023` | Missing `Keel.lock`. |
| `E6024` | Package is not present in the lockfile. |
| `E6025` | Publish conflict; published versions are immutable (identical re-publish is a no-op). |
| `E6026` | Invalid semantic-version requirement. |

## 13. Current limitations and validation status

The following are explicit limitations, not implied future guarantees:

- No stable cross-backend ABI/layout guarantee for arbitrary L++ aggregates. The `extern "C"` scalar/`Str` subset is the only documented imported-C surface.
- No general function-pointer/callback ABI, C varargs, C aggregate ABI, safe raw-pointer type, or compiler-verified foreign ownership. A pointer stored in `Int` is an unsafe-by-convention handle.
- No enforced `pub` privacy boundary; module global names are currently flat for duplicate checking.
- No exponent operator, general source pattern-matching grammar, one-element tuples, tuple arity outside 2–4, or arbitrary f-string expressions.
- No generalized lifetime system, mutable borrowed slices, general slice escape/thread transfer, async I/O reactor, fairness/work-stealing guarantee, or general data-race-freedom guarantee.
- WASM cycle collection is absent; LLVM managed-value/cycle-collector and Windows LLVM runtime parity are incomplete. Cross-OS sysroots and a complete Android toolchain are not provided by the target aliases.
- The builtin ABI catalogue is larger than the current implementation/backend subset. A registered builtin name or generated ABI row is not evidence of backend support.
- The C binding/translation project is experimental and its narrow profiles do not translate arbitrary C or all of SQLite.
- The current LSP advertises more than it semantically implements; see §11.1.
- The repository capability file reports a 2026-10-20 fast-gate/corpus run; that is historical context, separate from the verification below.

**Verification on 2026-10-23 (Rust 1.98.0):** `cargo fmt --all -- --check` passes, and `cargo test --workspace --locked` passes with 537 tests passed, 0 failed, and 0 ignored (115 test-result summaries, including doc-test targets). The full workspace test run includes Cranelift imported-extern and C-string ABI tests.

A native SQLite smoke also passed through the direct compiler/linker path: `main.lpp` imported a separate `bindings.lpp` declaring `sqlite3_libversion() -> Str` and `sqlite3_libversion_number() -> Int`, linked against the installed SQLite 3.46.1 library, and printed `3.46.1` and `3046001`. A same-file smoke also passed with both the direct and host linkers. This verifies those calls, imported declaration resolution, library propagation, and the `Str` result path; it does **not** verify SQLite database/statement handles, pointer-to-pointer APIs, parameter binding, or a complete database integration. The earlier `Unsupported(UnresolvedName)` failure was not reproduced in this reduced cross-module smoke; do not treat that as proof that every form of the earlier scenario is fixed.

## 14. Implementation references

| Area | Primary source/reference |
|---|---|
| Lexer/parser/tokens/literals | [`crates/lpp-frontend/src`](crates/lpp-frontend/src) |
| HIR, lowering, module graph, names | [`crates/lpp-hir/src`](crates/lpp-hir/src) |
| Type inference/checking/traits | [`crates/lpp-types/src`](crates/lpp-types/src) |
| MIR construction, verification, interpreter | [`crates/lpp-mir/src`](crates/lpp-mir/src) |
| Optimization and ownership proofs | [`crates/lpp-passes/src`](crates/lpp-passes/src), [`crates/lpp-ownership/src`](crates/lpp-ownership/src) |
| Backend target/API | [`crates/lpp-codegen-api`](crates/lpp-codegen-api) |
| Hosted runtime, ARC, strings, I/O | [`crates/lpp-runtime/src`](crates/lpp-runtime/src) |
| Builtin ABI registry | [`abi/builtins.toml`](abi/builtins.toml), [`docs/reference/BUILTINS_GENERATED.md`](docs/reference/BUILTINS_GENERATED.md) |
| External C ABI | [`documentation/EXTERNAL_C_ABI.md`](documentation/EXTERNAL_C_ABI.md) |
| Capability index | [`documentation/CURRENT_CAPABILITIES.md`](documentation/CURRENT_CAPABILITIES.md) |
| Runtime panic contract | [`documentation/PANIC_AND_ERRORS.md`](documentation/PANIC_AND_ERRORS.md) |
| Keel manifest/workspaces/registry | [`crates/keel`](crates/keel), [`crates/lpp-pm`](crates/lpp-pm), [`docs/rewrite/KEEL.md`](docs/rewrite/KEEL.md) |
| C analysis/binding tool | [`packages/lpp-bindgen/docs/CONFIG.md`](packages/lpp-bindgen/docs/CONFIG.md) |
