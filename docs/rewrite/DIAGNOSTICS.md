# Keel: `outdated` + `why` + `verify` — contract

**Status: APPROVED (contract-first)** — 2026-09-06
**Slice:** 9 (+10: verify) — read-only diagnostics (the "most loved" command tier)
**Error block:** E6xxx (package manager)

---

## 1. `keel outdated [package]` — what could be newer

Read-only companion to `keel update` (the `cargo outdated` analog): for
each **registry-sourced** package in `Keel.lock`, compare the locked
version against what the registry offers **today**. Never writes anything.

### Statuses

| Status | Meaning |
|---|---|
| `up to date` | locked == latest non-yanked offered. |
| `update available` | a newer non-yanked version exists (`latest` column shows it). |
| `yanked` | the **locked** version was yanked after the lock was written — a real warning (the pinned version is gone from the candidate set; `keel update` will move it). |
| `removed` | the package name no longer exists in the registry at all. |

(Note: a "locked version newer than any offered one" state is impossible —
a non-yanked locked version is a member of its own latest set — so no
extra status is needed.)

- Yanked versions are excluded from the `latest` computation (consistent
  with `fetch`/`update`).
- `keel outdated <name>` restricts to one package; a name not in the lock
  → **E6024**.
- No `Keel.lock` → **E6023** (`run `keel fetch`` hint).
- No registry configured:
  - lock has registry packages → the standard "no registry configured" error;
  - lock has none → empty table + "nothing to check".
- Registry is synced once. Output sorted by package name. Exit code 0
  (diagnostics are not failures).

### Output

```
+---------+---------+---------+------------------+
| package | current | latest  | status           |
+---------+---------+---------+------------------+
| math    | 1.0.0   | 1.2.0   | update available |
| stats   | 0.1.0   | 0.1.0   | up to date       |
+---------+---------+---------+------------------+
1 package(s) with an update available — run `keel update`
```

`latest` = `—` for `yanked`/`removed`. The summary line prints only when at
least one package is `update available`, `yanked`, or `removed`.

---

## 2. `keel why <name>` — why is this package here

Read-only, **offline** (lock-driven): explain the path(s) by which
`<name>` entered the resolved graph (the `cargo tree -i` analog).

- `<name>` must be in `Keel.lock` → else **E6024**.
- Edges: the lock's dependency names (package P lists dep d ⇒ edge
  `d ← P`). Chains are followed from every **workspace member** (root)
  down to the target.
- Output = one line per distinct chain, target-first, deterministic
  (members in discovery order; a package's deps in declaration order):

```
why: math v1.0.0 (registry)
math ← app (direct)
math ← stats ← app
```

- `(direct)` marks a member's own direct dependency.
- If a member depends on the target only transitively, only that chain
  prints. Cap: first 32 distinct chains (deterministic), then
  `… and N more`.
- No lock → **E6023**.

---

## 2b. `keel verify` — supply-chain integrity audit

Read-only: does the registry actually serve **exactly the bytes my
lockfile promises**, for every locked registry package?

- The **lockfile is the source of truth** — not the registry index. Each
  artifact is fetched (which verifies artifact == *index* checksum) and
  then re-hashed against the **locked** checksum. A tampered registry
  (index rewritten to point at an attacker's blob) is caught here, where
  a plain fetch would not.
- Registry is synced once (so blobs are present); path/member packages are
  skipped (local, not content-addressed) and counted.

### Statuses

| Status | Meaning |
|---|---|
| `ok` | artifact present, SHA-256 == locked checksum. |
| `missing` | blob (or the version in the index) absent from the registry clone. |
| `mismatch` | artifact present but hashes differently from the locked checksum (corruption / tamper / drift). |
| `removed` | package name no longer in the registry. |

- Output: table `package | version | sha256 (first 12 of the locked one) |
  status`, sorted by name, then a summary line.
- **Exit code is meaningful for CI**: all `ok` → 0; anything else →
  non-zero (the table is still printed).
- No lock → E6023; no registry configured → the standard registry error.

```
+---------+---------+--------------+--------+
| package | version | sha256       | status |
+---------+---------+--------------+--------+
| math    | 1.0.0   | 1e9f3ff43b5e | ok     |
| stats   | 0.1.0   | 9a3c22d0771f | ok     |
+---------+---------+--------------+--------+
2 package(s) verified OK
```

## 3. Error codes (new)

| Code | Meaning |
|---|---|
| E6023 | `Keel.lock` not found in the workspace (hint: `keel fetch`). |
| E6024 | Package `<name>` is not in `Keel.lock`. |
| E6025 | `keel publish` of an already-published version (`same = true` = identical artifact; `false` = conflicting artifact — versions are immutable). |

## 3b. `keel publish` merge fix (found by the diagnostics live demo)

The live demo exposed a real bug: `keel publish` **clobbered** the package's
existing index entry (only the new version was written), silently "removing"
old versions from every consumer's lockfile. Now `keel publish` **merges**:
- new version appended to the existing entry (append-only history, sorted);
- republish of an existing version → **E6025** (identical artifact = no-op
  republish; different artifact = immutability violation).

Pure helper `merge_publish(existing, name, new_version)` is unit-tested; the
full append→republish-rejected lifecycle is covered end-to-end through the
binary (`registry_gate::publish_command_appends_versions_end_to_end`).

## 4. Implementation notes

- New module `crates/keel/src/commands/diagnostics.rs` (both commands;
  they share the lock-loading helper). Wired in `cli.rs` + `lib.rs`.
- `outdated` takes `Option<&Registry>` like `update` (offline-aware).
- `why` is pure lock arithmetic (no registry).
- **No new external crates.** Table via `tabled` (existing keel dep);
  chains via plain strings.

## 5. Gate tests (`crates/keel/tests/diagnostics_gate.rs`)

1. Seed git registry: `math 1.0.0`, `stats 0.1.0` (deps: `math ^1`).
2. Project `app` → `math ^1`, `stats >=0.1`; fetch.
3. `outdated` → both `up to date`.
4. Publish `math 1.2.0` → `outdated` → math `update available` (latest
   1.2.0), stats unchanged; summary line present.
5. Publish `math 1.2.0` yanked → `outdated` → math back to `up to date`
   (1.0.0 is the latest non-yanked).
6. Yank `math 1.0.0` → `outdated` → math `yanked`; `outdated math`
   filters; `outdated ghost` → E6024; no lock dir → E6023.
7. `why math` → both chains (`← app (direct)` and `← stats ← app`).
   `why stats` → single direct chain. `why ghost` → E6024.
