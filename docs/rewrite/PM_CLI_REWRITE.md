# PM/CLI Rewrite — design & slice plan

Goal: a fresh, modern `lpp` command surface (clap + tables) and a rewritten
package manager, with an in-memory hot KV index. The old monolith path
(`src/pm.rs` + `src/legacy_driver.rs`) stays as a fallback until each command
is proven — **incremental takeover**, matching the repo's existing
parallel-rewrite pattern.

## The hot cache (behind a trait)
The hot layer is a small in-memory KV cache (`InMemoryKv`, a deterministic
`BTreeMap`) behind the `KvCache` trait. It is an *index over* the durable blob
store + lock — cheap to rebuild, never the source of truth — so a cache bug can
never take down `lpp build`. (An earlier vendored cache engine was removed as
over-engineered for a git-decentralized, zero-infra registry; recoverable from
git history if ever needed.)

## Layering
| Layer | Durable? | Backing |
| --- | --- | --- |
| Downloaded package blobs (content-addressed) | yes | `~/.cache/lpp/...` on disk |
| Lock file (dependency resolution) | yes | persistent lock |
| Metadata index (name → versions → deps) | hot | in-memory KV |
| Build-fingerprint → artifact map | hot | in-memory KV |

The hot layer is an **index over** the durable layer: cheap to rebuild, never
the source of truth. That is exactly the shape a KV cache fits — it is the
*cache* layer, not the registry and not the durable lock.

## Error block
`E6xxx` is the package-manager block (`E5xxx` = codegen, `E9xxx` = driver).

## Dependencies (new, user-authorized by the "modern CLI + Table" directive)
- `sha2` — content addressing (slice 1).
- `clap` (derive) + `tabled` — the modern CLI and table output (slice 2).

## Slices
1. **`lpp-pm` storage core** — content addressing, the durable blob store, and
   the hot KV cache with the `KvCache` trait. **[done.]**
2. **`lpp-cli`** — clap subcommands + table rendering, with `--cache-backend`.
3. **Wire into the `lpp` binary** — the new surface takes over, old path as
   fallback.
4. **[done] In-memory KV backend** — `InMemoryKv` (a deterministic `BTreeMap`)
   implements `KvCache`; `--cache-backend memory|auto` selects it (`auto` =
   memory). No external deps.
