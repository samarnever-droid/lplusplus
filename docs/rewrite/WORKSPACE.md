# Slice 5 — Workspace / monorepo model (path deps, workspace lock, parallel DAG)

> One model for single-repo AND monorepo (KEEL.md). A single repo is a
> 1-package workspace; a monorepo is an N-package workspace with path deps.
> Same code path, no special-casing.

## Manifest changes

```toml
# root Keel.toml — workspace root
[package]                      # OPTIONAL — absent = "virtual" workspace
name = "root-app"
version = "0.1.0"

[workspace]
members = ["crates/*", "apps/demo"]   # relative dirs; ONE `*` wildcard allowed
```

- `[workspace]` present → this dir is the workspace root.
- Root package (if any) is an implicit member. `members` add directories,
  each of which must contain a `Keel.toml` with a `[package]` (E6017 if not).
- Wildcard: exactly one `*` spanning a single path segment (`crates/*`).
  Anything else → E6020. A pattern that matches nothing is OK (0 members).
- A `Keel.toml` with `[package]` and no `[workspace]` → standalone package
  (today's behavior, unchanged). It is a 1-member workspace for planning.
- `Dependency::Detailed.version` becomes optional (path deps don't need a
  version; `{ features = [...] }` = any version). `version()` falls back to `"*"`.
- Duplicate member package names → E6018.

## Path dependencies

```toml
[dependencies]
math = { path = "../crates/math" }
```

- Resolved on the filesystem, never against the registry.
- The target must be a workspace member, else E6019 (add it to `members`).
- If a dep name equals a member name, it is always treated as the local
  member (the registry is not consulted for it).

## Discovery (`lpp_pm::workspace::Workspace`)

`Workspace::discover(start)` walks UP from `start` to the nearest
`Keel.toml` with a `[workspace]`; else `start` itself must be a package.
Yields `root` dir + ordered `members` (root package first, then `members`
declaration order). A sub-member dir discovers its root workspace, so
`keel build` from any member builds the whole workspace.

## Build plan + parallel DAG

`Workspace::build_plan()` → topological LAYERS of members (Kahn) over the
path-dep edges, deps first: `[[d], [b, c], [a]]` for a→(b,c)→d.
Cycles → E6016. Within a layer, members build **concurrently**
(`std::thread::scope`, std-only); layers run in order. The table output is
sorted by (layer, name) so output is deterministic.

- Single-member workspace: identical behavior to today (member-local
  `target/`, same table columns + a package column).
- Multi-member: shared `target/` at the workspace root (cargo-style).

## Workspace lock

`keel fetch` (no name) in a workspace resolves the UNION of all members'
registry requirements (non-path deps only; member-named deps are local) and
writes ONE `Keel.lock` at the root. Members appear in the lock with
`source = "root"` (root package) / `source = "path"` (no checksum).
Compatible requirements across members unify to one version (the resolver
reuses its existing conflict path — E6013 when incompatible).

## Errors (new)

| Code | Error |
| --- | --- |
| E6016 | `WorkspaceCycle` — dependency cycle among members |
| E6017 | `MemberNotFound` — a `members` entry has no package manifest |
| E6018 | `DuplicateMember` — two members share a package name |
| E6019 | `PathDepOutsideWorkspace` — path dep target is not a member |
| E6020 | `BadMemberPattern` — unsupported `members` glob |
