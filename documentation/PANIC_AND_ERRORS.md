# L++ panics and error semantics — the current contract

_Last audited: 2026-10-20. L++ has two error channels: **values**
(`Result` + `?`, for expected failures) and **panics** (for contract
violations — bugs). This file records the common no-unwind/exit contract
and the backend behaviors that have been verified. It does not claim that
every failure mode has identical diagnostics; see the parity table below._

## 1. The panic contract

A panic:
1. flushes stdout,
2. prints the banner below to **stderr** (host runtimes; freestanding
   min runtimes exit silently — they have no stderr):
   ```
   ===================================================================
   💥 L++ RUNTIME PANIC
   ===================================================================
   Reason: <reason>
   ===================================================================
   ```
3. **exits the whole process with code 101** (`std::process::exit(101)`
   / `exit(101)` / `ExitProcess(101)`).

There is **no stack unwinding**: destructors on the panicking path
do not run, other threads are not notified, and no `catch`/recovery
exists. This is deliberate — leak-on-process-death is memory-safe,
and unwinding through FFI frames would not be. A panicking program
is a *bug being reported*, not a recoverable state; use `Result`
for anything a caller should handle.

## 2. What panics (defined, never a hardware trap)

| Condition | Reason string | Where enforced |
|---|---|---|
| List/slice/string index out of bounds | `list index out of bounds: index <i>, len <n>` (family) | Runtime ops |
| Integer division or modulo by zero | `integer division by zero` | Guard emitted for Cranelift, LLVM, and WASM; Cranelift/WASM runtime-tested locally, LLVM execution pending a Clang-enabled run |
| Slice view of a dead source (generation mismatch) | slice validation | Runtime ops |
| Null list / invalid handle builtins | per-op reason | Runtime ops |
| Allocation failure | `out of memory ...` family | ARC allocator |
| Explicit `assert` failure | assert reason | `assert` builtin |

The MIR oracle models each of these as a typed `InterpreterError`
(e.g. `DivisionByZero`); native code must reach the same outcome
(exit 101) rather than a signal. Regression gate:
`integer_division_by_zero_is_the_defined_panic` (phase5f).

## 3. What does *not* panic (defined wrapping/IEEE semantics)

Integer arithmetic is **wrapping** (two's complement), matching the
oracle's `wrapping_*` ops exactly:
- `+`, `-`, `*` wrap on overflow; `INT_MIN / -1 == INT_MIN`;
  `INT_MIN % -1 == 0`; shifts mask the amount.
- Float arithmetic is IEEE-754: `x / 0.0` is `±inf`, `0.0/0.0` is
  `NaN` — no panic.

## 4. Backend parity status

| Backend | Div/mod-by-zero | Notes |
|---|---|---|
| Cranelift | ✅ defined panic (101), runtime-tested locally | Guard + `lpp_panic_div_zero`, wrapping `INT_MIN/-1` |
| LLVM | ⚠️ guard implemented; runtime validation pending | Explicit zero guard before `sdiv`/`srem`; substitute divisor 1 for `INT_MIN/-1`; conditional test skipped locally because `clang` is unavailable |
| WASM | ✅ defined exit (101), runtime-tested locally | Guard calls WASI `proc_exit(101)` before `i64.div_s`/`i64.rem_s`; explicit `INT_MIN/-1` wrapping path |

## 5. Driver behavior

`lpp run` propagates the child's exit code; a child killed by a
signal is reported as `128 + signal`, **never 0**
(`child_exit_code`, pinned by driver unit tests). Corpus/fuzz
harnesses rely on this: a crashing program can no longer masquerade
as a passing one.
