# L++ Feature Requests — Driven by SamarDB

**Filed:** 2026-08-21
**Source project:** `C:\Users\khati\Documents\antigravity\epic-hubble\samardb`
**L++ version probed:** v1.0.0 (Pure Native AOT, Cranelift)

SamarDB is a PostgreSQL-grade relational engine written in pure L++ (11,229 LOC,
25 modules, 18 test binaries). It is the largest real L++ program in existence
and therefore the best available source of truth for what the language is
missing. Every item below is backed by an executed probe, not speculation.

---

## Part 1 — SamarDB Status (verified 2026-08-21)

| Metric | Value |
|---|---|
| L++ source | 11,229 LOC across 46 `.lpp` files |
| Modules | 25 (`bytes` → `safety`, per ADR 0001–0016) |
| Test binaries | 18 |
| Assertions passing | **514** |
| Assertions failing | **1 (intermittent)** |
| Phases complete | 0–15; Phase 16 (bitmap index scan) next |

Feature surface already working: slotted-page heap, ARIES WAL + REDO recovery,
MVCC + CLOG + vacuum, Lehman-Yao B-link tree, SQL parser, Volcano executor,
Selinger cost-based optimizer, PostgreSQL wire protocol v3, Raft consensus,
2PL + Cahill SSI, hash aggregates, DDL catalog, FK/CHECK/UNIQUE constraints,
double-write buffer, savepoints, chaos/fault injection.

### The one failure is an L++ bug, not a SamarDB bug

`test_phase14.exe` (constraints/FK) **fails intermittently — ~25% of runs**,
either segfaulting (exit 139) or panicking with
`list index out of bounds: index 0, len 0`, or silently truncating stdout
mid-suite:

```
exit codes over 20 runs:  0 0 0 0 0 0 0 0 0 139 139 139 0 0 0 0 0 139 0 0
assertions reached:      20 20 16 20 20 20 20 20 20   9   8   8 20 20 20 16 9 20 20
```

The failing assertion is `constraint_validate_unique`, which is trivially
correct L++. Root cause is item **B1** below. `docs/memory/05_Active_Intent.md`
claims "497/497 (100%)" — that number is stale and was recorded on a lucky run.
**SamarDB currently has no trustworthy green suite, and cannot get one until B1
is fixed.**

---

## Part 2 — Correctness bugs (P0 — these block SamarDB)

### B1. ARC use-after-free on multi-field struct alias rebuild ⚠️ **top priority**

Reduced repro committed as `tests/arc_multi_field_alias_rebuild.lpp`.
Reproduces **30/30 runs**.

```lpp
struct Mgr:
    a: List[Int]
    b: List[Int]
    c: List[Int]

def mgr_new() -> Mgr:
    return Mgr(list_new(), list_new(), list_new())

def add_b(m: Mgr, v: Int) -> Mgr:
    mut nb := list_new()
    mut i := 0
    n := len(m.b)
    while i < n:
        list_push(nb, m.b[i])       # read loop over an aliased field
        i = i + 1
    list_push(nb, v)
    return Mgr(m.a, nb, m.c)        # >=2 fields aliased from the argument

def main():
    mut m := mgr_new()
    list_push(m.a, 7)
    mut k := 0
    while k < 40:
        m = add_b(m, k)             # reassign releases old Mgr
        k = k + 1
    print(list_get(m.a, 0))         # PANIC: len 0 — a's body was freed
```

Assigning over `m` releases the old `Mgr`, and that release frees the list
bodies that the *new* `Mgr` still owns. All three conditions are required —
remove any one and the bug vanishes:

1. **≥2 fields aliased** from the argument (1 aliased field is handled correctly)
2. **a read-loop over another aliased field** (`m.b[i]`) inside the rebuild
3. **the struct originates from a constructor helper**, not an inline literal

That last condition points at return-slot / escape handling: `mgr_new()`'s
freshly-allocated fields appear not to be marked as escaping through the return
slot, so the later release path treats them as owned-and-dead.

This matters far beyond one test. §12.1 of `LPP_SYNTAX.md` names
"Functional State Threading" as *the* core SamarDB idiom, and it is exactly this
shape. SamarDB threads state through ~200 such rebuild functions. Any of them
can corrupt memory.

### B2. Enum variant with a `Str` payload kills the process on match

```lpp
enum E2:
    A(v: Int)
    B(msg: Str)

match E2.B("hello"):
    A(v):
        print_str("A\n")
    B(m):
        print_str("  B=" + m + "\n")   # never runs; process exits silently
print_str("done\n")                     # never runs
```

`Int` payloads bind fine (`A(5)` → `A=5`). A `Str` payload terminates the
process with **exit 0 and no diagnostic** — the worst possible failure mode.

This single bug is why SamarDB contains **zero** enums and zero `Result` values
across 11k lines: `Result[T, Str]`, the natural error type for a database, is
unusable. Instead every fallible operation returns `Tuple[State, Bool]` and
throws the error message away. Fixing B2 is the prerequisite for SamarDB having
real error reporting.

### B3. `?` operator does not propagate `Err`

```lpp
def chain(x: Int) -> Result[Int, Str]:
    a := half(x)?
    b := half(a)?          # returns Err("odd") for x=6
    return Result.Ok(b)
```

`chain(8)` → `ok 2` correctly. `chain(6)` should surface `err odd`; instead the
process exits silently. Entangled with B2 (the payload is `Str`), but worth a
separate test once B2 lands.

### B4. `map_has` violates the `Bool` ABI — ✅ **FIXED 2026-08-21**

```lpp
print_str("has: " + bool_to_str(map_has(m, "zero")) + "\n")
```

```
[L++] cranelift backend compilation error: define_function:
  Compilation(Verifier(VerifierErrors([VerifierError {
    location: inst13, context: Some("v12 = call fn3(v11)"),
    message: "arg 0 (v11) has type i64, expected i8" }, ...
```

Two distinct defects, both fixed:

1. **Verifier failure.** `map_has` is `-> Bool` in the type table but its C
   definition returns `int64_t`, so its result reached `bool_to_str`'s `i8`
   parameter with the wrong Cranelift type. Fix: `coerce_args_to_signature()` in
   `src/backend/cranelift/lower.rs` now widens/narrows every `BuiltinCall`
   argument to the callee's declared width. Narrowing to `i8` goes through a
   nonzero test rather than `ireduce`, so a truthy value whose low byte is zero
   (e.g. `256`) stays `true` — matching how `if` already treats integers.

2. **Silent wrong answers.** Making it compile exposed the worse half:
   `bool_to_str(map_has(n, 6))` printed `true` for a *missing* key.
   `builtins.rs` declares `lpp_bool_to_str` with the I8 tag, but both C
   definitions took `int64_t`. An `i8` argument only defines the low byte of the
   register, so the callee's `val ? ...` branched on 56 bits of garbage. Fixed by
   changing the C signature to `int8_t` in `runtime/lpp_str.c:260` and
   `runtime/windows_x86_64_min.c:640`.

Audit result: `lpp_bool_to_str` was the **only** mismatch. The other five
I8-tagged builtins (`lpp_print_bool`, `lpp_list_get_bool`, `lpp_list_push_bool`,
`lpp_list_set_bool`, `lpp_slice_get_bool`) already agree with their C
prototypes. Note `file_exists` carries a stale `// I8 tag for bool` comment next
to an I64 tag — the tag is correct, the comment is not.

### B8. Runtime object cache ignores `#include`d sources — ✅ **FIXED 2026-08-21**

Found while fixing B4: the B4 runtime fix appeared to do nothing across three
rebuilds. `cached_runtime_object()` in `src/pm.rs` keyed the cache on
`lpp_runtime.c`'s path, size, mtime and compiler name only. `lpp_runtime.c` is
an amalgamation that `#include`s the whole `runtime/` tree, so editing
`runtime/lpp_str.c` left the key unchanged and the **stale object kept being
linked**. Any runtime edit silently no-ops until something unrelated touches the
amalgamation.

Fixed with `hash_local_includes()`, which folds the contents of every
transitively quoted `#include` into the key. Workaround for older builds:
`LPP_NO_RUNTIME_CACHE=1`.

Related: `runtime/lpp_runtime_min.obj` and `runtime/windows_x86_64_min.obj` are
checked into the repo and are months stale. Consider removing them, or at least
documenting that they are not the objects a normal build links.

### B5. Traits do not work at all

```lpp
trait Sized2:
    def byte_size(self) -> Int
```

```
error[E0003]: Undeclared identifier 'byte_size'
```

A bodyless trait method declaration fails semantic analysis, so §8 of
`LPP_SYNTAX.md` (traits, impl blocks, generic impls) is entirely
non-functional. SamarDB hand-rolls per-struct `*_encode`/`*_decode` free
functions in place of a `Serializable` trait.

### B6. Turbofish `::<T>` is documented but unimplemented

```lpp
lpp_print_int(identity::<Int>(9))
```

```
error[E0002]: Expected ')' after arguments
```

Bare generic structs (`Box(42)`) and inferred generic calls (`identity(7)`)
both work; only the explicit form is missing. Either implement it or cut it
from §6.6 of the syntax reference.

### B7. `match` arms reject qualified variant paths

`LPP_SYNTAX.md` §5.5 shows bare arms (`Ok(val):`), and only bare arms parse.
The natural qualified form fails:

```lpp
match s:
    TxStatus.InProgress:      # error[E0002]: Expected ':' after match arm
```

`parser.rs:934` accepts only a single `Ident`. Bare variants are ambiguous when
two enums share a variant name, and there is no `_` wildcard arm and no
exhaustiveness check. Accept `Enum.Variant`, add `_`, and check exhaustiveness.

---

## Part 3 — Missing features SamarDB needs (P1)

### F1. Fixed-width integer types — `u8..u64`, `i8..i64`

Every on-disk format in `docs/format/` is byte-exact, but L++ offers only
`Int` (i64). Consequences today:

- `bytes.lpp` hand-rolls big-endian codecs from `buf_get8` byte arithmetic
  (`(b0 * 256) + b1`) because there is no typed width to lean on.
- Overflow wraps silently — `9223372036854775807 + 1` yields
  `-9223372036854775808` with no trap. For LSN and TxnId arithmetic, which
  underpin invariants I5 (LSN monotonicity) and I6 (REDO idempotence), a
  silent wrap is a durability bug.

Wanted: fixed-width scalars with explicit, documented truncation semantics, and
a checked/trapping arithmetic mode for release builds.

### F2. Unsigned semantics — logical shift and unsigned compare/divide

`>>` is arithmetic-only: `-1 >> 1` is `-1`. There is no `>>>`, no unsigned
comparison, and no unsigned division. CRC32C, page checksums, and hash-bucket
selection are all naturally u32/u64 operations. `buf_get32le` correctly returns
`4294967295` for `0xFFFFFFFF`, so values above `i63` do enter the language and
then cannot be shifted or compared correctly.

### F3. Atomics with memory ordering

Zero `atomic_*` builtins exist (confirmed by grep over `src/builtins.rs`).
Needed: `atomic_load`, `atomic_store`, `atomic_add`, `atomic_cas` on 32/64-bit
with `acquire`/`release`/`seq_cst`. Without these, the buffer-pool pin counter,
LSN allocator, and CLOG cannot be made concurrent — `concur.lpp` implements
2PL and SSI against structures it has no way to protect.

### F4. Multi-executor / thread-per-core runtime

No `spawn_on`, `runtime_start`, `mutex`, or `rwlock` builtins. The executor is
single-threaded cooperative. A database that cannot use more than one core has
a hard performance ceiling, and the head-to-head benchmarks in
`docs/format/headtoheadbench.md` are measured with one core against
PostgreSQL's many. Blocked on F3.

### F5. `Map` iteration

`map_keys` / `map_values` / any iterator is absent — the full Map surface is
`map_new`, `map_put`, `map_get`, `map_has`, `map_remove`, `map_len`
(+`_str`/`_arc` variants). A hash map you cannot enumerate cannot back a
catalog scan, a `GROUP BY`, or a hash join. `agg.lpp` reimplements a bucket
table over `List` as a result, and `catalog.lpp` does linear scans over lists
where it wants a keyed lookup.

Also: `map_get` returns `0` for a missing key, indistinguishable from a stored
`0`. Callers must pair every `map_get` with a `map_has`, doubling the hash cost.
Return `Option[V]` once B2 makes enums usable.

### F6. Struct layout control — `repr(packed)`, `align(n)`

`layout.rs` aligns dynamically to 8 bytes with no override. Disk structures
(32-byte page header, 40-byte WAL header, 24-byte MVCC header) must be
serialized field-by-field through `bytes_set*` calls instead of being mapped
directly over a buffer. That is both slower and a place for the code and
`docs/format/*.md` to silently disagree.

### F7. Slices that can cross function and struct boundaries

Slice views are first-tier only: they cannot be returned, stored in a struct,
or captured. This is why `Bytes` is a hand-rolled `(handle, offset, length)`
triple rather than a real slice type, and why every accessor re-does bounds
arithmetic against `b.offset`.

### F8. Error propagation on builtins instead of sentinel returns

`bytes_get8` returns `0` on out-of-bounds. `buf_*` accessors silently accept
bad offsets. `map_get` returns `0` for missing. For an engine whose first
invariant is data safety, silent-wrong-value is the wrong default. Once
`Result[T, E]` works (B2/B3), the durable-I/O and buffer builtins should carry
real error codes.

---

## Part 4 — Performance and ergonomics (P2)

### F9. In-place mutation to retire "functional state threading"

**Struct field assignment and by-reference mutation already work** — verified:

```lpp
def bump(c: Counter):
    c.n = c.n + 1        # mutates the caller's struct
```

SamarDB does not use this. It threads state through `Tuple[State, Bool]`
returns, allocating a fresh struct per operation on every hot path — buffer
pool fetch, WAL append, B-tree descent. This is the single largest performance
item available, and it is a documentation problem, not a compiler one:
`LPP_SYNTAX.md` §12.1 actively teaches the slow pattern as the house idiom.
Recommend documenting `&mut`-style parameter mutation as the default and
demoting state threading to "use only where you need a snapshot". Note this
also reduces exposure to B1, whose trigger is the rebuild pattern.

### F10. String temporaries leak

`PLAN_AND_TODO.md` already records this: `lpp_str_concat` results sit on a
separate allocation path from struct ARC. SamarDB builds log and error strings
per operation, so this leaks under sustained load — precisely what
`test_stress_load.exe` is meant to certify.

### F11. Symbolized panic backtraces

Panics print raw addresses:

```
Stack Backtrace:
  [ 0] 0x00007FF703D91742
```

No symbols, no file/line. Diagnosing B1 required bisecting a 288-line test by
hand. Symbolize against the emitted binary, or at minimum print the L++
function name.

### F12. Stdout is lost on abnormal exit

Truncated runs of `test_phase14` show 16 of 20 assertions with exit 0 — buffered
stdout discarded on the abnormal path. Flush on panic and on exit; a test
harness that silently under-reports is worse than one that crashes.

### F13. Container and sort builtins

Missing: `list_insert`, `list_remove`, `list_reserve`/capacity control,
`list_sort`. `bench.lpp` hand-writes integer sorting for percentile
calculation; `opt.lpp` needs sorted access for join ordering. Reserve matters
for the buffer pool, which knows its size up front.

### F14. Deterministic time and RNG injection

Rule 4 in `docs/memory/06_Feedback_Rules.md`: "Modules do not access the OS
clock, RNG, network, or disk directly; all operations use injectable
interfaces." `time_ms()`, `random()`, and `random_seed()` are global builtins,
so SamarDB enforces this by convention only. A seedable, injectable clock and
RNG would let the deterministic simulator actually be deterministic.

### F15. SIMD vectors

No `simd_*` builtins. Relevant to CRC32C, page-comparison, and the vectorized
aggregate paths in `agg.lpp`. Lowest priority here.

### F16. No way to write to stdout without a trailing newline

`print_str` appends `\n` unconditionally, and the full output surface is
`print`, `print_str`, `print_int`, `print_float`, `print_bool` — all
line-terminated. So every `print_str("...\n")` in SamarDB emits a **spurious
blank line**, and building a line from parts requires concatenating the whole
thing into one temporary first (feeding F10's leak).

Wanted: `write_str` / `print_raw` with no implicit newline. Cheap to add, and it
would let the test harnesses emit compact output.

---

## Part 5 — Suggested order of work

1. **B1** — memory corruption in the language's own advertised core idiom. Nothing else can be trusted until this is fixed. Repro is committed and deterministic.
2. **B4** — one-line ABI fix; audit all `Bool`-returning builtins alongside it.
3. **B2 → B3** — unlocks `Result`/`Option` and real error handling; then F5's `Option[V]` and F8 become possible.
4. **F12, F11** — make failures observable before touching anything else.
5. **F9 documentation change** — largest performance win per unit of effort, no compiler work required.
6. **F1, F2** — correctness of the on-disk formats and LSN arithmetic.
7. **F5** — unblocks catalog scans, `GROUP BY`, hash joins.
8. **B5, B6, B7** — either implement or delete from `LPP_SYNTAX.md`; shipped docs currently describe features that do not exist.
9. **F3 → F4** — the concurrency story, in that order.
10. Remainder as capacity allows.

### Documentation accuracy

`LPP_SYNTAX.md` documents traits (§8), turbofish (§6.6), and qualified match
arms (§5.5) that do not work, and teaches the slow state-threading idiom
(§12.1) while omitting the by-reference mutation that does work. Any fix pass
should correct the reference alongside the compiler.

---

## Appendix — Probe files

Reduced repros live in `tests/arc_multi_field_alias_rebuild.lpp` (B1, committed).
The remaining probes were run ad hoc; each snippet in this document is
self-contained and compiles with
`lpp <file>.lpp && ./<file>.exe`.

Note for anyone reproducing on Windows: `vswhere.exe` must be on `PATH` or the
MSVC environment probe fails and linking dies with
`Failed to execute host linker 'cl.exe': program not found`.

```
export PATH="$PATH:/c/Program Files (x86)/Microsoft Visual Studio/Installer"
```
