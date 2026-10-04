# Slice 6 — Fingerprint incremental builds + the Delta layer

> "Delta" = the differential layer that decides *what* to rebuild between
> two builds. Backed by durable fingerprints, with an in-memory hot KV layer
> (KEEL.md).

## Fingerprint (`lpp_pm::fingerprint`)

Per (member, target):

```
fp = sha256(
  lpp_bin_path + lpp_mtime +        # compiler identity (rebuild ⇒ new mtime)
  target_triple +
  manifest_toml +
  sorted[(relpath, sha256(bytes))] of every .lpp under the member (minus target/),
  sorted[(dep_name, fp_of_dep)] of its PATH deps     # ← transitive invalidation
)
```

Because a member's fingerprint folds in its path deps' fingerprints, a
change to a dep automatically dirties every transitive dependent — no
separate bookkeeping. `compute_member_fps(ws, lpp_bin)` returns
`BTreeMap["member|target", FingerprintEntry]` in one pass (deps first).

## Durable store

`<workspace-root>/target/.keel/fingerprints.toml` (TOML, std-only —
lpp-pm already owns `toml`; no new deps):

```toml
version = 1
[entries."math|host"]
fingerprint = "abc…"
built_at = "2026-09-06T09:00:00Z"
```

`FingerprintStore::load/save/upsert/prune` — `prune` drops entries for
members that no longer exist. Load errors (missing/corrupt) = "cold"
(everything rebuilds), never a failure.

## Delta (`lpp_pm::delta`)

`Delta { added, changed, removed }` = the diff between the stored set and
the current `compute_member_fps` set (by key, then fingerprint).
`invalidate(delta, dep_graph)` = the changed/added/removed members plus
their transitive dependents (reverse edges) — the *rebuild set*, used for
the build report ("delta: changed d → rebuilding d, b, c, a").

**Hot KV seam:** when a cache backend is active, the per-member fingerprint
entries are also mirrored into the hot KV cache (keys `fp/<member>|<target>`),
so the in-memory layer holds the diff state; the durable TOML remains the
source of truth (the KV cache is in-memory only).

## `keel build` integration

For each (member, target): `fp == store[fp]` ⇒ **CACHED** (lpp not invoked,
artifact reused); else **BUILD** (lpp runs, store updated). Table columns:
`package | target | status (BUILD|CACHED|FAIL) | time (ms)`. The delta
summary prints first when anything changed.

## `keel cache clean` (finally wired)

Removes the global cache dir (`~/.cache/keel` or `$XDG_CACHE_HOME/keel`) —
registry clone, blobs, hot cache — and prints what it removed (entries +
bytes). This is the cargo-cache-clean analog (the durable *project*
fingerprints live in the workspace `target/`, untouched).

## Errors (new)

| Code | Error |
| --- | --- |
| E6021 | `FingerprintStore` — fingerprints file unreadable/unparseable (treated as cold) |
