# Keel — the L++ build & package manager

Mission: a legendary, most-loved, *fast* package manager for L++ — unified
single-repo and monorepo, scaling to 1M+ LOC — powered by a content-addressed
cache + fingerprint incremental builds.

## Keel ≠ lpp (the Cargo / rustc split)
The `lpp` binary is the **compiler** (rustc-analog). **Keel** is the **build &
package manager** (cargo-analog) — a separate tool that manages projects,
dependencies, and the global cache, and *drives* `lpp` to build. `keel build`
is the manager telling the compiler to build.

## One model for single-repo AND monorepo
- **Package** — a unit of code + a manifest (name, version, deps, targets).
- **Workspace** — 1..N packages sharing a root manifest + config + one
  `Keel.lock`. A single-repo is a 1-package workspace; a monorepo is an
  N-package workspace with path deps. Same code path, no special-casing.

## Why it's fast (and stays fast at 1M LOC)
| Mechanism | What it buys |
| --- | --- |
| Content-addressed global cache (`lpp-pm::DiskBlobStore`) | skip re-download/refetch; shared across projects |
| Fingerprint incremental builds (src+deps+features+compiler+target → key) | rebuild only what changed |
| **Hot KV index** (`lpp-pm`, in-memory) | instant resolution + cache-hit checks |
| Parallel DAG builds | independent packages build concurrently |
| **Delta** (incremental layer) | apply/recompile only the diff between builds |

**Delta** — the differential layer that drives incremental rebuilds; it diffs
the stored fingerprints against the current set to compute the rebuild set
(changed/added/removed members plus their transitive dependents).

## Layering (durable vs hot)
| Layer | Durable? | Backing |
| --- | --- | --- |
| Downloaded blobs / build artifacts | yes | `~/.cache/keel/...` on disk |
| `Keel.lock` (resolution) | yes | the workspace lock |
| Metadata index (name → versions → deps) | hot | in-memory KV |
| Build-fingerprint → artifact map | hot | in-memory KV |

## Crate structure
- `crates/keel` — the Keel manager: CLI (clap) + tables + orchestration.
- `crates/lpp-pm` — the core engine: content addressing, blob store, KV cache
  (done).
- `lpp` (existing) — the compiler, driven by Keel.

## Slices
1. **[done]** `lpp-pm` storage core (content addressing, blob store, KV cache).
2. **[done]** `keel` CLI: the full command surface — `new`/`init`, `add`/
   `remove`, `fetch` (one + all → `Keel.lock` via the resolver), `search`,
   `publish`, `build`/`check`/`run`/`test` (driving `lpp`), `cache`
   (`--cache-backend memory|auto`).
3. **[done]** Package model + `Keel.toml` manifest parsing, single-repo.
4. **[done]** Git-decentralized registry: sparse `index/` + content-addressed
   `blob/<sha256>` in a git repo; `fetch` = clone + offline read (SHA-256
   verified), `publish` = commit + push; system `git` (zero new deps).
5. **[done]** Monorepo: path deps, one `Keel.lock`, parallel DAG build
   (`docs/rewrite/WORKSPACE.md`).
6. **[done]** Fingerprint incremental builds + the Delta layer +
   `keel cache clean` (`docs/rewrite/DELTA.md`).
7. **[done]** Dependency linking: path deps staged into `.lpp_packages/`
   so `import <dep>` compiles — zero lpp changes
   (`docs/rewrite/DEP_LINKING.md`).
8. **[done]** `update` (full re-resolve + single-package with pinning,
   yank-safe, E6022 for gone pinned versions, offline no-op) + `tree`
   (offline dependency graph, lock-driven)
   (`docs/rewrite/UPDATE_TREE.md`).
9. **[done]** Read-only diagnostics: `outdated` (lock vs registry:
   up-to-date / update available / yanked / removed, E6023/E6024) +
   `why <pkg>` (offline dependency chains, `cargo tree -i` analog)
   (`docs/rewrite/DIAGNOSTICS.md`). This slice also fixed `keel publish`
   clobbering a package's version history (now append-only, immutable
   versions, E6025).
10. **[done]** `verify` — supply-chain audit: every locked registry
    package's bytes re-fetched and re-hashed against `Keel.lock`
    (ok / missing / mismatch / removed table, CI exit code, E6009/E6023).
    Live-proved: honest registry → exit 0; tampered registry (evil bytes
    pushed under the locked checksum) → `mismatch`, exit 1.
11. **[done]** Build-time dependency staging: every compile command
    (`build`/`check`/`run`/`test`) calls one `stage_all_deps(ws, reg)` —
    path deps **and** registry deps staged into `.lpp_packages/` (managed,
    pruned, idempotent on `version`+`checksum`), registry artifacts
    re-verified against the lock at build time (tampered registry refused,
    E6009). `keel test` runs from the workspace root and discovers tests
    across all members. Path-only workspaces stay fully offline.
    `scripts/demo_pm_lifecycle.sh`: full publish→add→fetch→run→update→run→
    verify→tamper→verify-fails lifecycle against a real git registry with
    the real lpp (prints 42, then 142 after the update; tamper caught).

## Dependencies (new, user-authorized by the "modern CLI + Table" directive)
- `clap` (derive) + `tabled` — the modern CLI + tables (slice 2).
- `sha2` (slice 1).
