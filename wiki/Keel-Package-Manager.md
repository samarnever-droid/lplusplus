# Keel Package Manager

**Keel** (invoked through `lpp` CLI commands) is the official dependency manager and build orchestrator for L++. It manages project manifests, dependencies, semantic version resolution, lockfiles, and package distribution.

---

## 1. Creating a Project

To initialize a new L++ project:

```bash
lpp new my_app
cd my_app
```

This creates a standard project layout:
```text
my_app/
├── lpp.toml          # Project manifest
└── src/
    └── main.lpp      # Entry point
```

Default `lpp.toml`:
```toml
[package]
name = "my_app"
version = "0.1.0"
authors = ["Your Name <you@example.com>"]
edition = "2026"
license = "MIT"

[dependencies]
```

---

## 2. Managing Dependencies

### Adding Dependencies
To add a package from the official registry ([registry.lplusplus.bond](https://registry.lplusplus.bond)):

```bash
lpp add json
```

This modifies `lpp.toml`:
```toml
[dependencies]
json = "1.2.0"
```

### Git Dependencies
You can also depend directly on a Git repository:
```toml
[dependencies]
http_client = { git = "https://github.com/example/http_client.git", branch = "main" }
```

### Local Path Dependencies
For monorepos or multi-crate workspaces:
```toml
[dependencies]
utils = { path = "../utils" }
```

---

## 3. The Lockfile: `lpp.lock`

When dependencies are resolved, Keel generates a deterministic `lpp.lock` file:
- Records exact semantic versions, resolved git commit SHAs, and integrity hashes.
- Ensures identical reproducible builds across all development machines and CI/CD environments.
- Always commit `lpp.lock` for executable applications.

---

## 4. Keel Commands Reference

| Command | Usage | Description |
|---|---|---|
| `lpp new <name>` | `lpp new my_library --lib` | Create a new binary or library project |
| `lpp add <pkg>` | `lpp add regex` | Add a dependency to `lpp.toml` |
| `lpp remove <pkg>` | `lpp remove regex` | Remove a dependency from `lpp.toml` |
| `lpp build` | `lpp build --release` | Compile all dependencies and the project |
| `lpp run` | `lpp run` | Compile and execute the main binary |
| `lpp test` | `lpp test` | Discover and run test suites |
| `lpp tree` | `lpp tree` | Print the visual dependency graph |
| `lpp publish` | `lpp publish` | Package and upload to `registry.lplusplus.bond` |

---

## 5. Publishing to the Registry

To publish a library package to the official registry:
1. Ensure your `lpp.toml` contains `name`, `version`, `description`, `license`, and `repository`.
2. Authenticate with your token:
   ```bash
   lpp login <api-token>
   ```
3. Run the publish command:
   ```bash
   lpp publish
   ```
Keel packages the source, computes the archive checksum, and verifies that the crate builds cleanly before uploading.
