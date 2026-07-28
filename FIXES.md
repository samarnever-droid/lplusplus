# Compiler fixes applied to L++ v4.3.0

Two genuine miscompilation bugs found while writing a SQLite-compatible engine
in L++ (`~/lppsqlite`, ~9,700 lines). Both are fixed in this tree; a third
issue reported earlier turned out **not** to be a compiler bug and is corrected
below.

Files changed:

- `src/mir/pass_copyprop.rs` — rewritten (bug 1)
- `src/main.rs` — `resolve_local_imports` + two new helpers (bug 2)

---

## Bug 1 — copy propagation destroys a variable that is still live

**Severity: silent wrong answers.** This is the serious one.

### Symptom

```lpp
def g() -> Int:
    return 42

def f() -> Int:
    first := g()
    mut cur := first     # cur aliases first
    cur = 99
    return first         # returns 0, should return 42
```

`mut y := x` silently corrupted `x` whenever `x` was an immutable local
initialised from a **function call** (or any expression containing one). With a
literal (`first := 7`) it did not reproduce, which made it look like an
aliasing quirk rather than an optimizer bug.

### Root cause

`pass_copyprop` folded the pattern `_tmp = expr; _dest = _tmp` into
`_dest = expr` unconditionally. That fold **rewrites the destination of the
defining instruction**, so `_tmp` is never assigned at all. It is only valid if
nothing else reads `_tmp` afterwards — but the pass never checked.

MIR for `f()` before the fix (`lpp f.lpp --dump-mir`):

```
  _0 = first        _1 = <temp>        _2 = cur
  bb0:
    _2 = 42          ; call result folded straight into `cur`
    _2 = 99          ; ...then overwritten
    return _0        ; _0 was NEVER written -> reads uninitialised memory
```

### Fix

Only fold when the source local is mentioned **exactly twice** in the entire
function — once by the definition being rewritten, once by the copy being
deleted. Any local mentioned a third time is read later, reassigned, or live
across blocks, so removing its definition would be observable.

A full mention-count is computed per function over every instruction operand,
rvalue and terminator before folding begins.

MIR after the fix:

```
  bb0:
    _0 = 42          ; `first` really is assigned
    _2 = _0
    _2 = 99
    return _0        ; 42
```

The safe fold is preserved: a genuine single-use temporary (e.g. `s = s + a*b`
inside a loop) is still collapsed, so the optimisation's original purpose is
intact.

### Verification

| Case | pristine v4.3.0 | fixed |
|---|---|---|
| `mut cur := first` after a call | `0` | **`42`** |
| 7 aliasing variants | `7 0 0 0 42 0 6000` | **`7 42 43 8 42 2 6000`** |

**Strongest evidence:** `~/lppsqlite` originally contained **110** hand-written
`mut x := 0` / `x = expr` workarounds for this bug. All 110 were reverted to the
natural `mut x := expr` form, and the whole suite still passes — 485 in-engine
assertions and 87 differential cases against real SQLite.

---

## Bug 2 — two modules may define the same name, silently

**Severity: silent wrong function called.**

### Symptom

```lpp
# ca.lpp
def shared() -> Int:
    return 111

# cb.lpp
def shared() -> Int:
    return 222

# main
import ca
import cb
def main():
    print(ca.shared())   # printed 222
    print(cb.shared())   # printed 222
```

Both calls resolved to whichever definition was linked last. When the two
functions had *different* return types the type checker produced a confusing
error blaming the wrong file ("Return type mismatch in function 'helper'");
when the signatures matched, it compiled cleanly and returned wrong data.

### Root cause

`resolve_local_imports` flattens every imported module's declarations into one
global `Vec<TopLevel>`. There is no per-module namespace, so duplicate names
collapse into a single symbol.

### Fix

Proper namespacing is a large language change. The safe fix is to make the
collision an error instead of silent misbehaviour: each imported declaration is
tagged with its originating module, and any name defined by two different
modules is reported.

```
error[E0005]: duplicate definition of 'shared': defined in both 'ca.lpp' and 'cb.lpp'.
  Imported modules share one global namespace in L++, so two modules cannot
  define the same function, struct, enum, const or type name.
  Rename one of them.
```

Applies to functions, structs, enums, consts and type aliases.

---

## Correction — the "loop-exit sentinel" was **not** a compiler bug

An earlier note claimed this idiom miscompiled:

```lpp
while j < n:
    if pred(c) == 1:
        j = j + 1
    else:
        j = n + 1000      # intended to force loop exit
if j > n:
    j = j - 1000
```

It does not. The generated MIR is correct; the **logic is wrong**. Setting
`j = n + 1000` exits the loop, but the trailing `if j > n: j = j - 1000`
restores `j` to exactly `n`, so a non-digit at position 2 of `"12a45"` yields
`5`, not `2`. An identical transcription in Python returns the same `5,5,5`.

The flag-based rewrite (`mut go := 1`) that "fixed" it was a genuine logic
correction, not a workaround. The claim of a second compiler bug was wrong and
is withdrawn.

---

---

## Build-speed work (same tree)

Two independent changes, both verified to leave output byte-identical.

### 1. Cache the compiled C runtime — `src/pm.rs`

The host linker passed `lpp_runtime.c` to `cc` on **every** link, recompiling a
~40 KLOC translation unit (it `#include`s the whole `runtime/` tree) each time.
That was ~180 ms of a ~200 ms link.

It is now compiled once into `$LPP_HOME/cache/lpp_runtime-<hash>.o` and reused.
The key covers the source path, size, mtime and compiler name, so editing the
runtime or switching compilers invalidates it. Compilation writes to a
pid-unique temp file and renames, so concurrent builds cannot see a partial
object. `LPP_NO_RUNTIME_CACHE=1` disables it; a failed compile falls back to
passing the `.c` directly.

### 2. Parallel machine-code generation — `src/backend/cranelift/compiler.rs`

`lower_functions` now runs in three phases:

1. **serial** — build Cranelift IR (must stay serial: it mutates the module,
   interning callee references and string-literal data objects)
2. **parallel** — `ctx.compile()` per function on a worker pool; this is the
   expensive half (regalloc + instruction selection) and each function is
   independent
3. **serial** — `define_function_bytes` in source order

Thread count comes from `available_parallelism()`, overridable with
`LPP_CODEGEN_THREADS`; modules under 8 functions stay serial because thread
setup costs more than it saves.

### 3. Deterministic output (prerequisite for the above)

While validating phase 2 I found the object file was **already
non-deterministic**: `MirProgram::functions` is a `HashMap`, so
`declare_functions` and `lower_functions` emitted symbols in a different order
on every run. Verified against a pristine build of upstream `13e73c2` — three
runs of the *same* compiler on the *same* input produced three different
objects.

Both loops now iterate in `FuncId` order. Output is byte-identical across runs
*and* across thread counts, which is what makes the parallel path safe to trust:

```
threads=1  fbe1681025e2af1e  fbe1681025e2af1e  fbe1681025e2af1e
threads=2  fbe1681025e2af1e  fbe1681025e2af1e  fbe1681025e2af1e
threads=4  fbe1681025e2af1e  fbe1681025e2af1e
```

### Measured (lppsqlite, 9.8 KLOC, 2-core container)

| Stage | before | after |
|---|---|---|
| codegen (`--emit-obj`) | 183 ms | **149 ms** |
| full build (codegen + link) | 380 ms | **174 ms** |

Roughly **2.2× faster end to end**; the runtime cache is the larger share. On
machines with more cores the codegen phase scales further.

---

## Regression testing

| Suite | Result |
|---|---|
| `tests/aot_parity.tsv` (upstream, 24 programs w/ expected stdout) | 24/24 pass |
| `~/lppsqlite` unit suites (11 suites, 485 assertions) | all pass |
| `~/lppsqlite` differential vs real `sqlite3` (87 cases) | 87/87 pass |
| ARC positive tests (`arc_borrowed_return`, `arc_branch_return`, …) | unchanged |

### Pre-existing failures (NOT caused by these changes)

`aot_reject_arc_cycle`, `aot_reject_list_arc_cycle` and `aot_reject_mut_closure`
are supposed to be *rejected* at compile time but are accepted. Verified against
a pristine build of upstream `13e73c2`: **all three are already accepted before
any of my changes.** The documented ownership-cycle rejection is not actually
enforced in v4.3.0. Left as-is — out of scope here, but worth filing upstream.

---

## Reproducing

```sh
cd ~/lplusplus
CARGO_TARGET_DIR=/tmp/lpp-target cargo build --release -j1 --bin lpp --bin lpp-link
cp /tmp/lpp-target/release/lpp{,-link} ~/lpp-toolchain/bin/

cd ~/bugs && lpp bug1.lpp  --linker host && ./bug1    # 42
cd ~/bugs && lpp bug1b.lpp --linker host && ./bug1b   # 7 42 43 8 42 2 6000
cd ~/bugs && lpp cm.lpp    --linker host              # duplicate-definition error
```

Build with `-j1`: the container has 2 GB RAM and parallel cranelift codegen
gets OOM-killed.
---

## Monomorphization completion

Generics were only half-implemented: the pass existed and handled the simplest
case, but four gaps made anything realistic either fail to compile or compile
to the wrong code. All four are fixed in `src/analysis/monomorph.rs`.

### 1. Generic structs as parameter types were never specialised

```lpp
struct Box[T]:
    value: T

def unwrap[T](b: Box[T]) -> T:
    return b.value          # error[E0004]: Cannot access field 'value'
                            #   on non-struct type Generic("Box",[TypeParam("T")])
```

Type-parameter inference only matched a bare `Type::Custom("T")`, so a
parameter declared `Box[T]` bound nothing. Replaced with structural
unification (`unify_type`) that recurses into generic arguments and recovers
bindings from an already-specialised argument's mangled name
(`Box__Int` -> `T = Int`). A concrete `Box[Int]` in a signature is now rewritten
to the nominal `Box__Int` and that struct instantiated on demand.

### 2. Arguments that were variables inferred as Int

```lpp
f := 1.5
identity(f)     # specialised as identity__Int, not identity__Float
```

`infer_simple_expr_type` only understood literals and fell back to `Int` for
everything else — silently producing a wrongly-typed specialisation that
surfaced later as a confusing error at the *use* site. The pass now tracks
local variable types (seeded from parameters, updated at each `let`) and also
recognises struct-constructor calls.

### 3. Generic templates were left in the AST

A template body such as `return b.value` where `b: Box[T]` mentions an unbound
`T` and is not compilable on its own. Templates are now pruned after
specialisation, keeping only the concrete copies.

### 4. Nested generic calls in specialised bodies

```lpp
def twice[T](x: T) -> T:
    return identity(identity(x))    # identity stayed generic
```

Specialised bodies are re-walked to a fixed point, so generic calls *inside*
them are specialised too. Combined with (3) this was essential: previously the
nested call named a template that no longer existed.

### Bonus: unsatisfied trait bounds are now reported

A violated bound used to `return` silently, leaving the call site naming a
pruned template — which surfaced as `Undeclared identifier 'make_noise'`. It is
now a real diagnostic:

```
error[E0003]: type 'Rock' does not implement trait 'Speak'
              required by generic parameter 'T' of 'make_noise'
```

A guard (`bindings_are_concrete`) also prevents specialising on a binding that
is itself a type parameter, which previously emitted nonsense like `Box__T`.

### Verification

| | |
|---|---|
| `cargo test` | 35 pass (3 new monomorphization regression tests) |
| `tests/aot_parity.tsv` | 25/25, including a new `generics_full.lpp` end-to-end case |
| `packages/lppsqlite` | 118/118 differential vs real sqlite3 |
| `packages/compresslpp` | 50 checks + 32 cross-verifications |

`generics_full.lpp` exercises generic functions at Int/Str/Float, generic
structs, generic structs as parameters, generic functions returning generic
structs, nested generic calls, and multi-parameter generics (`Pair[A, B]`).

### Still not supported (at the time of that change)

Explicit turbofish, generic enums and generic methods in `impl` blocks were
listed here as unsupported. All three are addressed in the follow-up section
below. Generic *trait implementations* (`impl Show for Box[T]`) remain
unsupported.

Nesting a generic struct inside itself (`Box(Box(5))`) does work — the
fixed-point loop resolves it, and is capped at 16 rounds to guarantee
termination.

---

## Enum payload representation, turbofish & generic methods

Chasing "finish monomorphization" surfaced a deeper defect: generic enums could
not be made to work because the *enum representation itself* was lossy, for
plain enums as much as generic ones.

### The representation bug

Every enum value was a single packed `i64`:

```
(tag << 32) | (payload & 0xFFFFFFFF)
```

The payload was masked to 32 bits, so anything wider was silently destroyed:

| Program | Before | After |
|---|---|---|
| `Msg.Text("hello")` then `match` | SIGSEGV (pointer truncated) | `hello` |
| `Num.N(5000000000)` | `705032705` | `5000000000` |
| `Num.F(2.5)` | rejected: "expects Float, got Int" | `2.5` |
| `Opt.Some(5)` + `Opt.Some("hi")` | SIGSEGV on the second | `5`, `hi` |

The wrong-number case is the dangerous one — no crash, no diagnostic, just a
different number than the program stored.

Enums are now ordinary ARC heap objects that reuse the existing struct
machinery. `typecheck.rs` gives each enum a field layout of `__tag` plus one
`__vN` slot per data variant, so each variant's payload keeps its declared type
at full width instead of being erased to a masked `i64`.

Three consequences had to be handled:

- **`semantic.rs`** bound every match arm as `Type::Int`. It now looks up the
  variant's declared payload type, which is what makes a `Float` or `Str`
  binding type-check.
- **The `?` operator** still read the packed form, and its error path emitted
  `Return`, so the ARC pass inserted `release(_2); return _2` — a
  use-after-free. `tests/test_try_operator.lpp` segfaulted on the `Err` path
  both before and after the representation change; it now returns `ReturnOwned`
  and prints `110` / `1`.
- **`Enum.Variant` resolution** used "this type has no fields" to distinguish an
  enum from a struct. That test silently became false once enums had fields, so
  the type checker now tracks enum names explicitly (`enum_names`).

### Turbofish

`identity[Int](42)` parsed but never resolved, so the call named a template
that monomorphization had already pruned and the user saw
`Undeclared identifier 'identity'`.

There is now an `Expr::GenericCall` node. The parser only reads `name[...]` as
type arguments when the brackets contain identifiers/commas **and** are
followed immediately by `(`; `xs[1]` and `s[2]` still parse as subscripts, which
`generics_turbofish.lpp` asserts. Monomorphization takes the bindings directly
from the written arguments and rewrites the node to a plain call, so no later
stage ever sees it. Mistakes are reported rather than inferred around:

```
error[E0003]: 'identity' expects 1 type argument(s) but 2 were given
error[E0003]: 'plain' does not take type arguments, or monomorphization could not resolve them
error[E0003]: type 'Rock' does not implement trait 'Speak' required by generic parameter 'T' of 'make_noise'
```

### Generic methods in `impl` blocks

A generic method was walked but never specialised, so it reached the backend
still generic and Cranelift failed its own verifier:

```
VerifierError { message: "arg 1 (v4) has type f64, expected i64" }
```

The parser already mangles impl methods to `Target_method`, so after that
rewrite they are just free functions. They are now registered as generic
templates, specialised per call site (`Holder_pick__Float`), and the templates
are pruned from the impl block. `h.pick(7)`, `h.pick("s")` and `h.pick(0.5)`
all work in one program.

### Missing runtime symbol

`float_to_str` linked under `--linker host` but failed under the default
internal linker with `unresolved GOT symbol 'lpp_float_to_str'`. The function
existed only in `runtime/lpp_str.c` (the host path) and was missing from
`runtime/linux_x86_64_min.c`. It is implemented there now, formatting by hand
and trimming trailing zeros since that build has no `snprintf`.

This was **pre-existing**, not caused by this work: a three-line program calling
`float_to_str` fails identically on an unmodified `b38019c`. It is fixed here
because it blocked `generics_full.lpp` in the parity suite.

### Verification

| | |
|---|---|
| `cargo test` | 38 pass (3 new: turbofish, turbofish arity, generic impl method) |
| `tests/aot_parity.tsv` | 30/30, with new `enum_payloads.lpp` and `generics_turbofish.lpp` |
| `scripts/check_safety_mission.sh` | pass |
| `packages/lppsqlite` | 485 unit checks + 118/118 differential vs real sqlite3 |
| `packages/compresslpp` | 59 unit checks + 38 cross-verifications |

### Still not supported

Generic trait implementations (`impl Show for Box[T]`) are still a parse error,
and inherent `impl Type:` blocks without a trait are not accepted — only
`impl Trait for Type:` parses. Inference remains local: an argument whose type
cannot be determined from a literal, a tracked local, or a struct constructor
still falls back to `Int`, though turbofish now provides an explicit escape
hatch when that guess is wrong.

Enum payloads are limited to **one field per variant**; `Pair(a: Int, b: Int)`
as a variant keeps only the first. The layout supports more (`__vN` could be
widened to `__vN_M`), but the constructor and match paths currently read a
single slot, so this is documented rather than silently half-working.

---

## ARC atomic contention

Every retain/release used an atomic read-modify-write unconditionally. An
atomic RMW is not costly because it is atomic — it is costly because it takes
exclusive ownership of a cache line. When two cores touch the same object's
refcount that line ping-pongs between them and throughput collapses.

Measured on this machine (2 cores, 20M retain/release pairs, same object):

| scenario | throughput |
|---|---|
| 1 thread, non-atomic | 225.9 M/s |
| 1 thread, atomic `ACQ_REL` (what L++ did) | 78.7 M/s |
| 2 threads, private objects | 156.1 M/s |
| 2 threads, **sharing one object** | 31.3 M/s |

Two threads sharing an object finish **slower than one thread**. The atomics
themselves cost 2.87x; the contention on top of that costs a further ~5x.

### What was rejected, and why

**Weakening the memory order** (relaxed retain, release/acquire-fence decrement)
is the textbook fix. Measured: **1.01x** — no effect. On x86 the `lock` prefix
dominates and the ordering flags are nearly free. Shipping it would have been
theatre.

**A per-object "is shared" flag** checked at run time is the other obvious
design. It is actively harmful: the check must load the flag from the very cache
line that is already contended, taking the shared 2-thread case from 0.487s to
**0.770s** — worse than plain atomics. Measured before it was written, not after.

### What was done

A whole-program MIR pass (`src/mir/pass_arc_local.rs`) proves whether the
program can ever create a second thread. When it cannot, codegen emits
`lpp_arc_retain_local` / `lpp_arc_release_local`, which drop the `lock` prefix
entirely. The decision is made at compile time, so it costs nothing at run time
and cannot regress the contended path.

This mirrors Rust's `Rc`/`Arc` split, but inferred rather than annotated —
L++'s philosophy is that the compiler proves ownership properties instead of
asking the programmer to declare them.

The proof is conservative. Any of the following keeps the atomic path:

- `Rvalue::SpawnThread` anywhere in the program;
- a call to the `lpp_thread_spawn` builtin;
- any `CallIndirect`, since the callee is unknown and might spawn;
- any `extern` block, since foreign code can spawn threads invisibly.

Verified by inspecting emitted relocations: an identical struct program uses
`_local` with no `spawn`, atomic with a `spawn`, and atomic with an `extern`
block.

Both runtimes gained the non-atomic pair. The freestanding runtime
(`runtime/linux_x86_64_min.c`, used by the *default* internal linker) is a
notable case: it exposes no thread primitive at all — no pthreads, no `clone` —
so a program linked against it cannot create a thread even in principle. Every
`lock xadd` it executed was pure overhead with nothing to synchronise against.

### Results

| | |
|---|---|
| Micro-benchmark, 1 thread | 78.7 -> 225.9 M/s (**2.87x**) |
| End-to-end L++ program, 3M ARC-heavy iterations | 0.0466s -> 0.0394s (**1.18x**) |

The end-to-end figure is much smaller than the micro-benchmark, and honestly so:
real programs also allocate, branch and do I/O, and ARC is only part of the
total. The 2.87x is the ceiling for the ARC operations themselves.

### Verification

| | |
|---|---|
| `cargo test` | 43 pass (5 new soundness tests for the pass) |
| `tests/aot_parity.tsv` | 31/31 (new `arc_local_refcount.lpp`) |
| `scripts/check_safety_mission.sh` | pass |
| `packages/lppsqlite` | 485 checks + 118/118 differential |
| `packages/compresslpp` | 59 checks + 38 cross-verifications |

`tests/arc_local_refcount.lpp` exercises the paths where a miscount would show
up as a double free or a leak: one object held through three live aliases, and
100 short-lived objects each aliased once. Its object file contains three
`retain_local` / three `release_local` calls and zero atomic ARC calls, and it
exits cleanly under both linkers — so the counts balance exactly.

### Not addressed

The gain applies only to programs that never spawn. A genuinely multi-threaded
L++ server still pays full atomic cost on shared objects — that needs biased or
deferred reference counting, which requires an owner field in the header and a
safe ownership-transfer protocol, and is a much larger change.

**Separately found, pre-existing:** several ARC-on-struct shapes segfault at
exit under the **freestanding** runtime (`runtime/linux_x86_64_min.c`, the
default internal linker). The host runtime is clean and the printed values are
correct in every case — the fault is in teardown. Confirmed triggers:

- returning a struct from a function;
- nested struct construction, `Outer(Inner(11), 1)`;
- taking a struct as a parameter and aliasing it inside the callee.

Minimal reproduction:

```
struct Inner:
    n: Int

def pass_through(i: Inner) -> Inner:
    copy := i
    return copy

def main():
    r := pass_through(Inner(23))
    print(r.n)          # prints 23, then SIGSEGV at exit
```

Verified pre-existing by stashing every change in this section, rebuilding
unmodified `13f3ef8`, and reproducing byte-identical failures. Not caused by
this work and not fixed here; it is very likely the same family as the
documented "returning a list created inside a function" hazard.

`tests/arc_local_refcount.lpp` deliberately avoids those shapes so it stays a
signal about ARC atomicity rather than a duplicate report of a known backend
fault. It passes under both linkers, and its object file contains three
`retain_local` and three `release_local` calls with zero atomic ARC calls.

---

## Move-out: transferring to a thread is not sharing

### The finding that redirected this work

The obvious home for this analysis is `analysis/escape.rs`, which classifies
bindings `Value`/`Arc`/`Arena` and has an explicit "Rule 4: crossing a
concurrency boundary" case. **That map is dead code.**
`run_arc_insertion_pass` takes it as `_escape_map` — underscore-prefixed,
unused — and decides everything from MIR `Ownership` instead. Its only real
consumer is the `--dump-escape` printer.

Verified rather than assumed:

```
$ grep -n "storage" src/main.rs        # the map analyze() returns
  Ok(storage) => {                      # bound
  for (id, class) in &storage {         # printed by --dump-escape
  run_arc_insertion_pass(&mut mir_program, &storage);   # ignored
```

An audit for the same pattern across `src/mir`, `src/backend` and
`src/analysis` found no other disconnected consumer — the two remaining
underscore params are unused arguments of a small local helper
(`get_root_binding`), not a severed pipeline.

So Rule 4 does classify a pure handoff as `Arc`, exactly as suspected, but
changing that classification would alter a diagnostic and nothing else. The
proof had to move to MIR, where retain/release are actually emitted.

### The distinction

Crossing a concurrency boundary is not sharing. If the spawning thread never
touches the value again there is one owner before the spawn and one after,
never two at once — that is a move, and nothing needs counting. Only when both
sides hold a live reference simultaneously is a refcount earning its keep.

MIR makes this decidable, because capture, spawn and later uses are all
explicit in one function body:

```text
    _env.cap_0 = borrow(_0)
    retain(_0)                  <-- second owner, for the thread
    _c = make_closure(f, [_env])
    _  = spawn_thread(_c)
    ...anything reading _0 after this point?...
    release(_0)
```

If nothing reads `_0` afterwards, the pair is pure overhead — a reference
created only to be destroyed, and on the atomic path each half is a locked RMW
on a contended line.

Measured on the two shapes:

| program | before | after |
|---|---|---|
| handoff (`j` dead after spawn) | 1 retain, 3 releases | **0 retains, 2 releases** |
| share (`j` read after spawn) | 1 retain, 3 releases | 1 retain, 3 releases (unchanged) |

### Conservatism

Every uncertainty keeps the retain:

- any read of the local after the spawn, in any reachable block;
- a spawn inside a **loop** is never eligible — a later iteration re-reads the
  same lexical binding, so "dead after this point" stops being meaningful.
  Cycles are excluded wholesale rather than reasoned about;
- the local must be released exactly once, so the removed pair is balanced;
- uses in branches that rejoin after the spawn count as uses.

### On use-after-move

No "use of moved value" diagnostic is needed, because this is a liveness
*proof* rather than a declaration. A binding is only treated as moved when no
later read exists; if one exists, the value stays shared and refcounted. That
inverts Rust's ordering — Rust declares the move then rejects later uses, L++
observes that there are no later uses. The case that would need the error is
simply classified as a share instead, so there is no soundness hole to close.

### Verification

| | |
|---|---|
| `cargo test` | 48 pass (5 new: handoff, later use, loop, successor-block use, non-spawn retain) |
| `tests/aot_parity.tsv` | 31/31 |
| `scripts/check_safety_mission.sh` | pass |
| `packages/lppsqlite` | 485 checks + 118/118 differential |
| `packages/compresslpp` | 59 checks + 38 cross-verifications |

Runtime soundness was checked directly: three moved payloads read inside their
spawned threads printed `11 22 33` on five consecutive runs. A premature free
would show as garbage or a crash. `tests/moveout_spawn.lpp` covers handoff,
genuine share, and a handoff inside a branch; it is host-linker only, since the
freestanding runtime has no thread primitive (`spawn` there fails to link), and
the parity harness requires both linkers to agree.

### Not addressed

This removes false positives — syntactic sharing that is semantically a
handoff. Genuinely shared mutable state across threads still pays full atomic
cost, exactly as it would in Rust with `Arc<Mutex<T>>`. The physics of a
contended cache line do not change.
