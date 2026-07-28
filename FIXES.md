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
