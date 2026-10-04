# Keel: `update` + `tree` — contract

**Status: APPROVED (contract-first)** — 2026-09-06
**Slice:** PM surface completion (post-slice 7)
**Error block:** E6xxx (package manager)

---

## 1. `keel update [package]` — refresh `Keel.lock`

Refresh the workspace lockfile against the git registry (the
`cargo update` analog).

### Semantics

| Form | Behavior |
|---|---|
| `keel update` | Re-resolve the **entire** workspace dependency graph to the latest semver-compatible versions. Prior lockfile choices are ignored (except for the diff display). |
| `keel update <name>` | Update **only** `<name>` to its latest compatible version. Every other already-locked package is **pinned** to its locked version (exactly one candidate is offered to the resolver). Newly declared dependencies resolve to latest. |

- Roots are built exactly like `keel fetch` (all members; path deps and
  member-named deps excluded from registry resolution).
- Candidate sets **exclude yanked versions** (also fixes the latent hole in
  `keel fetch` — its candidate builder previously could offer yanked versions).
- If no registry is configured (no `--registry` / `KEEL_REGISTRY`):
  - `keel update` stays offline-safe: locked packages are offered back at
    their locked version (idempotent no-op), new registry deps fail with the
    usual resolver error.
  - `keel update <name>` for a registry package errors with the standard
    "no registry configured" message.
- If a **pinned** locked version is no longer available in the registry
  (removed or yanked): **E6022** —
  `locked version {version} of {name} is no longer available; run `keel update``
  (without a package) to re-resolve.

### Output

Writes `Keel.lock` at the workspace root (E6013 convention), then prints a
diff table of what changed:

```
updated Keel.lock:
  package  old      new
  math     1.0.0    1.1.0
  stats    —        0.2.0
  oldlib   0.9.0    —
```

With no changes: `Keel.lock is up to date.`

Rows are sorted by package name; `—` = absent. Members (source `root`/
`path`) are excluded from the diff (they never change).

### Error codes (new)

| Code | Meaning |
|---|---|
| E6022 | A pinned locked version is gone from the registry (single-package update). |

---

## 2. `keel tree` — the resolved dependency graph

Print the dependency tree of every workspace member (one project = one
tree). Reads `Keel.lock` when present, so it works **offline**.

### Node labels

`name vVERSION (tag)` where tag ∈:
- `(member)` — a workspace member (version from its manifest)
- `(path)`   — a path dependency
- `(registry)` — from `Keel.lock`
- `(unlocked)` — declared but not in `Keel.lock` (version shown as `?`)

Transitive edges come from `Keel.lock` (`LockedPkg.deps`); without a lock,
only declared (direct) dependencies are shown, plus a footer hint to run
`keel fetch`.

### Shape & safety

- Indented with box drawing: `├──`, `└──`, `│`.
- Deterministic: members in discovery order, children in declaration order.
- Cycle-safe: a name already on the current branch prints as
  `name vVERSION (registry) (cyclic)`.

### Output example

```
myapp v0.1.0
├── calc v1.2.0 (registry)
│   └── mathx v0.9.1 (registry)
└── stats v0.2.0 (registry)
    └── mathx v0.9.1 (registry)
```

---

## 3. Implementation notes

- New modules: `crates/keel/src/commands/update.rs`, `tree.rs`; wired in
  `cli.rs` + `lib.rs` dispatch.
- `workspace_roots(ws)` extracted from `commands/registry.rs` (shared by
  `fetch`, `update`).
- Resolver unchanged: `resolve_workspace` + a candidate-provider closure
  implements both update modes (pin = offer one candidate).
- **No new external crates.** Tables via the existing `tabled` dep of the
  keel crate; tree via plain strings.

## 4. Gate tests (`crates/keel/tests/update_tree_gate.rs`)

1. Seed git registry: `math 1.0.0`, `stats 0.1.0` (deps: `math ^1`).
2. Workspace `app` depends on `math ^1`, `stats ~0.1`.
3. `fetch` → lock: math 1.0.0, stats 0.1.0.
4. Publish `math 1.1.0` + `stats 0.2.0` → `update` → lock bumped, diff
   printed; second `update` → "up to date".
5. Publish `math 1.2.0` + `stats 0.3.0` → `update math` → math 1.2.0,
   stats **pinned** at 0.2.0.
6. Yank math 1.1.0 → `update math` picks 1.2.0 (yanked excluded).
7. `tree` shows members + locked versions + tags; transitive edge under
   stats.
