# Keel Package Manager

**Keel** (accessible both as standalone `keel` and via `lpp <cmd>` when no `.lpp` source file is passed) is the official package and workspace manager for L++. It manages `Keel.toml` manifests, semantic version resolution, `Keel.lock` lockfiles, local caching, and decentralized Git registry publishing.

---

## 1. Creating a New Project

```bash
lpp new my-app
cd my-app
```

Generated layout:
```text
my-app/
├── Keel.toml          # Project manifest
└── src/
    └── main.lpp       # Program entry point (`def main()`)
```

Default `Keel.toml`:
```toml
[package]
name = "my-app"
version = "0.1.0"
edition = "2024"

[dependencies]
```

> Note: Staged dependency copies placed inside `.lpp_packages/` use internal `lpp.toml` descriptors for the compiler's module resolver; user-authored projects and workspaces always use `Keel.toml` (see [`specs.md`](../specs.md) §10).

---

## 2. Managing Dependencies

### Adding Dependencies
```bash
lpp add my-dep
```
Validates the package against the configured registry index (`llppregistry` by default) and records the constraint in `Keel.toml`:
```toml
[dependencies]
my-dep = "^0.2"
```

### Building, Running & Testing
```bash
lpp install            # Resolve & fetch dependencies, write Keel.lock
lpp build              # Compile package binaries/libraries
lpp run                # Build & run package binary
lpp test               # Run package tests
```

---

## 3. Lockfiles (`Keel.lock`) & Workspaces

When resolving dependencies, Keel writes a deterministic `Keel.lock` containing exact resolved versions and SHA-256 content hashes for offline verification.

Workspaces can span multiple member crates in `Keel.toml`:
```toml
[workspace]
members = ["crates/*"]
```
Path dependencies must resolve to members within the workspace; raw `git = "..."` dependency tables in `Keel.toml` are currently rejected by manifest validation.

---

## 4. Common Package Commands

| Command | Example | Description |
|---|---|---|
| `lpp new <name>` | `lpp new demo-app` | Create project directory with `Keel.toml` and `src/main.lpp` |
| `lpp init` | `lpp init` | Initialize `Keel.toml` in the current directory |
| `lpp add <pkg>` | `lpp add lpp-json` | Add a registry dependency to `Keel.toml` |
| `lpp remove <pkg>` | `lpp remove lpp-json` | Remove a dependency from `Keel.toml` |
| `lpp update` | `lpp update` | Refresh `Keel.lock` from registry index |
| `lpp tree` / `lpp why` | `lpp why lpp-json` | Inspect dependency tree and reverse dependencies |
| `lpp verify` / `lpp cache` | `lpp verify` | Verify cached SHA-256 blobs and workspace lockfiles |

---

## 5. Decentralized Git Registry (`llppregistry`)

Keel uses a decentralized Git-backed package index (defaulting to `llppregistry`) with SHA-256 archive verification instead of a centralized API-token server. There is no `lpp login <api-token>` workflow. Publishing uses `keel publish` / `lpp publish` with standard Git authentication (SSH/HTTPS credentials) against the configured registry remote; published versions are immutable (`E6025`).
