# Slice 7 — Dependency linking (path deps become importable code)

> Slices 5/6 gave path deps **build order + invalidation**. This slice makes
> them **functional**: `import calc` in a dependent compiles against the
> member's actual source. Zero `lpp` changes — we drive lpp's existing
> module resolver.

## How lpp resolves `import calc` (existing, untouched)

`resolve_module_filepath(module, base_dir)` tries, in order:
1. local to the entry file (`<dir>/calc.lpp`, `<dir>/src/calc.lpp`, parent dirs, `./src/calc.lpp`)
2. the built-in stdlib (known names take precedence)
3. **`.lpp_packages/<pkg>/` — CWD-relative**: `lpp.toml` manifest
   (`[package] name/version/entry`); the `entry` field names the module
   file inside the package dir; otherwise `<pkg>.lpp` / `src/<pkg>.lpp` /
   `src/main.lpp`.

## What Keel does

Before building, `keel build` **stages** every path dep of every member
into `<workspace-root>/.lpp_packages/<dep>/`:

```
<root>/.lpp_packages/calc/
    lpp.toml          # [package] name, version, entry = "src/lib.lpp", managed = "keel"
    src/ -> <dep-dir>/src      # symlink (Unix; copy fallback elsewhere)
```

- `import calc` from any member → lpp finds `.lpp_packages/calc` (lpp runs
  with **cwd = workspace root**), parses `lpp.toml`, compiles `src/lib.lpp`
  (through the symlink) into the program — flattened at the AST level by
  lpp's existing `resolve_local_imports`. Transitive deps work because each
  path dep is staged for itself.
- **Idempotent**: a correctly staged entry is left alone; a stale one is
  recreated.
- **Pruning**: only directories whose `lpp.toml` carries `managed = "keel"`
  are removed when no member references the dep anymore. Foreign
  `.lpp_packages` entries (old PM, hand-rolled) are never touched.
- Known-stdlib name collision (a member named `math`) resolves to the
  stdlib, matching lpp's own precedence — documented, not fixed.

## Command wiring

- Job runner signature gains the cwd: `Fn(member, cwd, lpp, rows)` — all
  `lpp` invocations run with `current_dir = workspace root` (keel build /
  check / run), so module resolution is deterministic regardless of where
  the user invokes keel.

## Tests

- `deplink_gate`: staging creates `lpp.toml` + `src` symlink with the
  right target; lpp is invoked with cwd = root; stale managed entries are
  pruned; unmanaged entries survive; transitive deps are staged.
- Live: `app` does `import calc; print(calc.square(7))` → the built binary
  prints `49` with the real `lpp`.

## Known lpp bug (pre-existing, NOT caused by Keel) — FIXED

Multi-argument `print` used to fail on the Cranelift backend with
`mismatched argument count for call ... got 2, expected 1` — reproducible
with a single file and **no** imports:

```lpp
def main():
    print("sum =", 2 + 2)
```

Root cause (lpp, not Keel): the shadow type checker skipped builtin arity
whenever a param was `Any` (print's only param), and the legacy MIR builder
then pushed all args into the single-operand `lpp_print_*` builtin. **Fixed**
in the lpp compiler: strict builtin arity is now enforced at type-check time
with a clear `error[E0004]`, plus a defense-in-depth guard in the MIR
builder. Single-argument `print` is unchanged.

## Slice 11 update (2026-09-06)

`build`/`check`/`run`/`test` now all call a single `stage_all_deps(ws, reg)`:
path deps **and** registry deps are staged before `lpp` runs, and each
registry artifact is re-hashed against the `Keel.lock` checksum at build
time (a tampered registry is refused, `E6009` — the same guarantee as
`keel verify`). `keel test` runs with cwd = workspace root and discovers
tests across all members. Full contract + live demo:
`DEP_LINKING.md` (repo root) and `scripts/demo_pm_lifecycle.sh`.
