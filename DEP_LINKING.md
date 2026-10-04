# Dependency Linking in L++ (Keel + lpp)

**Status: WORKING — no lpp changes needed.**
**Date:** 2026-09-06 (Slice 7)
**Verified end-to-end:** `import calc; print(calc.square(7))` → `49`

---

## The problem

`import calc` in a program looks for `calc.lpp` or `src/calc.lpp` next to the
entry file. A git-registry package (fetched by `keel fetch`) lands in a
**global cache** (`~/.keel/registry/...`), not next to the source. So `import
calc` fails with `error[E0007]: could not resolve import: calc`.

lpp's import resolution (in `src/legacy_driver.rs: resolve_module_filepath`):
1. `<entry_dir>/<name>.lpp`
2. `<entry_dir>/src/<name>.lpp` (+ parent fallbacks)
3. CWD-relative `./src/<name>.lpp`
4. **`.lpp_packages/<name>/` in CWD** ← the hook we use

Step 4 reads `lpp.toml` inside the folder and uses its `entry` field (default
`src/<name>.lpp`).

---

## The solution: stage path deps into `.lpp_packages/`

Keel makes **workspace path dependencies** visible to lpp by materializing a
copy (symlink farm) inside the project:

```
myapp/
├── lpp.toml
├── src/
│   └── main.lpp          # import calc; ...
└── .lpp_packages/
    └── calc/
        ├── lpp.toml      # generated: name/version + entry = "src/lib.lpp"
        └── src/          # SYMLINK → /abs/path/to/calclib/src
            └── lib.lpp
```

- `lpp.toml` is **generated** by keel (`managed = "keel"` marker) with
  `entry = "src/lib.lpp"` — lpp never needs to understand a bare `lib.lpp`.
- `src/` is a **symlink to the dependency's absolute src path**, so edits to
  the dependency source are picked up live (no copy, no staleness).
- `.lpp_packages/` is gitignored in the generated `lpp.toml`.
- Idempotent: re-running staging only updates the manifest; prunes managed
  entries whose dependency was removed from `lpp.toml`.
- **Transitive path deps self-stage**: when keel compiles a package that has
  its own path deps (e.g. a library), it stages those into
  `<dep>/.lpp_packages/` first — lpp resolves `import` relative to each
  module's own directory, so transitive imports find their deps in place.

### Why a generated lpp.toml instead of just a symlink?

`import calc` needs a **file** to load (`.lpp`). The package's entry is
`src/lib.lpp` (a library, no `main()`), which lpp cannot link as an
executable. The staging manifest gives lpp an entry that resolves to the real
source while keeping the package a proper library. Keel's build step still
links the final binary from the **member** entry, not from `.lpp_packages/`.

---

## What keel does (per build/check/run/test)

1. Parse `lpp.toml` → find `[dependencies]` entries with `path = "..."`.
2. Validate each path exists and contains an `lpp.toml` (E6019: path deps must
   be members).
3. For each, stage into `<workspace_root>/.lpp_packages/<name>/`:
   - `lpp.toml` (managed marker, entry pointing at `src/lib.lpp`)
   - `src` → symlink to `<dep>/src`
4. Run lpp with CWD = workspace root, so step-4 resolution finds the staged
   folder.

## What was NOT needed

- **Zero lpp changes.** The `.lpp_packages/` fallback already exists in
  `resolve_module_filepath` (src/legacy_driver.rs). Keel just uses it.
- No global cache involvement — this is for **workspace path deps**.
  Git-registry deps (slices 8+) will need the same staging from the cache.

## Slice 11 (2026-09-06): every compile command resolves deps (path + registry)

Contract: **`build`, `check`, `run`, and `test` all stage dependencies before
invoking `lpp`**, so `import <dep>` works from any of them — not just
`build`.

Two gaps closed:

1. **`check`/`run`/`test` did not stage path deps.** Only `build()` called
   `stage_path_deps`. A fresh `keel run` (no prior `build`) on a project with
   path deps failed with `E0007 could not resolve import`. Now a single
   `stage_all_deps(ws, reg)` is called by **every** compile command.

2. **Registry deps were never staged.** `import mathx` from a git-registry
   package failed: the artifact sat in the global cache, but lpp's resolver
   only looks in `.lpp_packages/`. Now `stage_registry_deps` fetches each
   locked registry artifact, **re-hashes it against the lockfile's checksum**
   (the same supply-chain guarantee as `keel verify` — a tampered registry is
   refused at build time, not just at audit time), and extracts it into
   `.lpp_packages/<name>/` (real files, managed by Keel, pruned like path
   deps). Idempotent: an entry whose staged `version`+`checksum` already
   matches is not re-fetched/re-extracted.

- `test` now runs with **cwd = workspace root** (was the member dir), so
  `.lpp_packages` and the shared `target/` resolve correctly in a monorepo.
- Staging needs a registry **only when** a member actually has a registry
  dep; path-only workspaces stay fully offline. A registry dep without a
  configured registry → the standard "no registry configured" error.
- Extraction uses the system `tar` (the artifact is a `tar.gz`), consistent
  with `publish`. **No new external crates.**

## Known bugs / limitations

| Issue | Status |
|---|---|
| lpp cannot link a bare library entry (`src/lib.lpp`) — needs a `main()` or `--emit-object` | Worked around (stage 3) |
| `lpp.toml` in a dir does NOT make it a package root (only `lpp package init` does) | Worked around (stage 2) |
| Multiple args to `print(...)` | **FIXED** (slice before 11): strict builtin arity at type-check + MIR guard. |
| `check`/`run`/`test` didn't stage path deps; registry deps were never staged | **FIXED** (slice 11): `stage_all_deps` in every compile command; registry artifacts verified against the lock and extracted to `.lpp_packages/`. |
| A parse error inside an **imported** module is rendered against the **entry** file (coordinates belong to the imported file) — misleading during dep debugging (lpp `render_error_string` reuses the entry filename for import-stage errors) | Known (lpp, pre-existing); tracked here. |

## Resolution contract (lpp side)

`lpp` resolves a third-party `import <name>` (in this order, local/stdlib
first):

1. `<entry dir>/<name>.lpp`, `<entry dir>/src/<name>.lpp`, parent variants,
   CWD `src/<name>.lpp` — local modules;
2. shipped stdlib;
3. **`.lpp_packages/<name>/`** (CWD-relative): if the dir has an
   `lpp.toml`/`lpp.json` manifest with an `entry` field, that entry file is
   used; otherwise fallback candidates (`<name>.lpp`, `src/<name>.lpp`,
   `src/main.lpp`, `main.lpp`).

Keel's registry staging therefore writes `.lpp_packages/<name>/lpp.toml`
with `entry = "src/lib.lpp"` (or `src/main.lpp`) — the staged `lpp.toml` is
a superset of the published `Keel.toml` plus `checksum`, `source =
"registry"`, and `managed = "keel"`. Path-dep staging copies real files and
a managed manifest the same way, so lpp never needs to know which kind of
dep it is loading.

Note: L++ requires typed signatures — `def double(x: Int) -> Int:` (no
untyped parameters).

## Verification (live, 2026-09-06)

`scripts/demo_pm_lifecycle.sh` exercises the full system against a **real
git registry** with the **real lpp compiler** (all steps pass, exit 0):

```
git registry → publish mathx 1.0.0 (real git commit)
→ keel new/add/fetch → keel run            (prints 42 — registry import staged+linked)
→ keel tree → keel outdated (up to date)
→ publish mathx 1.1.0 → keel outdated (update available)
→ keel update (diff table + lock refresh) → keel run  (prints 142 — new version)
→ keel why → keel verify (ok)
→ TAMPER the registry (evil bytes under the locked checksum, pushed via git)
→ keel verify → `mismatch`, exit 1   ← tamper caught
→ restore + keel build (build OK)
```

Point `KEEL_REGISTRY` at any git URL (GitHub/Gitea) with write access to
run the same demo against a real host.

Full workspace test suite: 598/598 passing (includes 7 dedicated
`staging_gate` tests: fresh run/check staging, registry staging verified
against the lock, idempotent re-staging, tampered-registry refusal at
build time, clean no-registry/no-lock errors, and `keel test` from a
virtual workspace root).
