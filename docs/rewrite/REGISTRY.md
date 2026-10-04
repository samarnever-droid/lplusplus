# The L++ Package Registry — architecture

**The registry is a git repository.** No server, no hosting fee, no credit
card — it lives on any git remote (GitHub free today, a self-hosted Gitea for
total control, or any machine). It is **decentralized**: anyone who clones it
holds a complete mirror, so there is no single point of failure. It is
**offline-first**: clone once, then `keel add`/`fetch` work with no network.
Content-addressed (SHA-256), checksummed, reproducible.

## Canonical source of truth (reconciling the five representations)

Five registry representations have accreted in the tree — the keel git client,
the Cloudflare worker, `registry/index.json`, `registry.toml`, and a duplicate
static index under `website/public/registry/` — but only **one** is canonical.
This is the decided architecture (v1/LegacyEngine is retired and is no longer a
reference):

| Representation | Role | Disposition |
| --- | --- | --- |
| **git registry repo** — sparse `index/<path>` + content-addressed `blob/<sha256>` (`lpp-pm::registry`, this doc) | **canonical source of truth** | keep — keel clones it (offline, SHA-256-verified) and publishes via `git commit`+`push` |
| `registry-worker/index.ts` (Cloudflare Worker, "v3.0") | HTTP layer for the website/discovery | **rewrite as a read-only mirror**: serve the sparse index + 302 to `blob/<sha256>`. It currently keeps packages/tokens in **in-memory edge maps** (ephemeral — publishes vanish on eviction) and never reads the git index, despite its docstring claiming git-as-source-of-truth. That is the gap to close. |
| `registry/index.json` ("v1.0", GitHub-Pages shape: `source`, `api`) | legacy static website index | **regenerate** from the canonical sparse index, or retire |
| `registry.toml` ("v2.0": `{git, path, version}`) | legacy static manifest | fold into the generated index; retire the separate shape |

**Decisions:** (1) the git repo is the single source of truth — decentralized,
free, no card, offline-first; the production registry remote is
`samarnever-droid/llppregistry`; (2) one registry version + one URL
(`registry.lplusplus.bond`); (3) the worker is a *mirror*, never an authority —
durable publish goes through git/Keel `push`, not an in-memory map; (4) downloads
are **content-addressed** (`/blob/<sha256>`), matching
`IndexEntry::download_url`.

### Deployed reality (read from the infra) — and what it forces

Verified by reading `wrangler.toml`, `registry-worker/`, `website/`, and
`.github/workflows/pages.yml`:

- **The website is GitHub Pages, not Cloudflare Pages.** `pages.yml` builds the
  Vite/React `website/` and pushes `dist` to the `gh-pages` branch
  (`samarnever-droid.github.io/lplusplus`). Only the *worker* is Cloudflare
  (`registry.lplusplus.bond`). The site is the HTTP **consumer** of the registry.
- **The site calls the worker over HTTP**: `GET /index.json`, `GET /stats`,
  `GET /search`, and `POST /auth/create-token`. So the worker cannot simply be
  deleted — it must keep serving those routes (as a read-only mirror).
- **Live route mismatch (bug):** the site calls `/auth/create-token`, but the
  worker only serves `/tokens` + `/api/v1/tokens` → token creation 404s today.
  Fix one side.
- **The worker has NO storage binding** — `wrangler.toml` declares no KV, R2, or
  D1. That is *why* `EDGE_PACKAGES`/`EDGE_TOKENS` are in-memory and ephemeral,
  and proof the worker structurally cannot be the source of truth.
- **Supabase is dead wiring.** `wrangler.toml` still sets `SUPABASE_URL`/
  `SUPABASE_KEY` ("Cloudflare Workers + Supabase Backend"), but neither the v3.0
  worker (in-memory) nor the website (`website/src/lib/db.ts` is browser
  **IndexedDB** for academy progress) uses it. Remove it.
- **`website/public/registry/index.json`** is a second copy of the v1.0 static
  index, served from GitHub Pages alongside the live worker → two read paths for
  the same data.

**Refined decisions (from the above):**
1. **One generator → one static index.** `scripts/generate_registry_index.py`
   generates the aggregated `index.json` from the canonical sparse `index/*`
   when present, otherwise validates the transitional aggregate
   `registry/index.json`; it writes identical output to `registry/index.json` and
   `website/public/registry/index.json`.
2. **The website reads the *static* generated index** (GitHub Pages / raw git)
   for the package list — reads no longer depend on the ephemeral worker. The
   worker stays only for `/stats` (live GitHub-Releases download counts) and as a
   convenience HTTP mirror of the same generated index.
3. **Publish is git, never the in-memory map.** keel `publish` = `git commit` +
   `push` (durable, decentralized). Browser/Worker publishing is retired until a
   future GitHub-App or PR-based flow exists.
4. **Auth:** Clerk is removed from the critical path. Publisher authority is git
   write access: SSH keys, deploy keys, GitHub PATs, or branch-protected pull
   requests. No package/token state is stored in Worker memory.
5. **Retire the dead config:** drop Supabase and Clerk vars from `wrangler.toml`;
   unify on one registry version + the content-addressed `/blob/<sha256>`
   download path.

### Worker v3.0 read end-to-end — the surface to mirror, the defects to dump

`registry-worker/index.ts` (853 lines, "v3.0.0 Pure Git + GitHub Releases") was
read in full. Its **read** surface and schemas are good and worth mirroring so
Keel and the rewrite worker stay drop-in compatible; its **write** path and
**auth** are non-functional and insecure, and are exactly what decisions 1–5
replace.

**Route table to keep (mirror these paths):**

| Route | Handler | Reality |
| --- | --- | --- |
| `GET /index.json`, `/registry/index.json`, `/` | `handleIndex` | manifest `{registry{…}, packages{name→pkg}}`, `max-age=30` |
| `GET /search?q=`, `/api/v1/search` | `handleSearch` | substring over name/desc/keywords/authors |
| `GET /stats`, `/telemetry` | `handleStats` | counts + live GitHub-Releases download totals (keep — the one genuinely live endpoint) |
| `GET /packages/:name` | `handleGetPackage` | one `RegistryPackage` |
| `GET /download/:name/:file` | `handleDownload` | **302 → `github.com/<repo>/raw/master/packages/:name/:file`** → replace with `blob/<sha256>` |
| `POST /tokens`, `/api/v1/tokens` | `handleCreateToken` | website calls `/auth/create-token` — **route mismatch**, fix one side |
| `POST /publish`, `/api/v1/publish` | `handlePublish` | **in-memory only** (defect 3) |
| `GET /health`, `/status` | inline | liveness |

**`RegistryPackage` schema to stay compatible with:** `name, version, description,
authors[], license, repository, git, path, source_url, dependencies[], keywords[],
features[], sha256, size, owner_email, organization, downloads, published_at,
versions{ver→{download_url, sha256, size, published_at}}`. Name rule
`^(?:@[a-z0-9_-]+/)?[a-z0-9][a-z0-9_-]{0,63}$` + a 21-name reserved set + SemVer-2.
The seed `OFFICIAL_PACKAGES` (lpp-graph, lreact, lpp-json, lpp-toml, lpp-semver,
lpp-sha256, lpp-math, lpp-strings, lpp-collections, lppsqlite, …) point at
`packages/<name>` / `stdlib/<file>` — these become the first git `index/` entries.

**Dump these (real defects, not mere gaps):**
1. **Hardcoded master admin token** — `verifyToken` accepts the literal
   `lpp_pub_518f8c2a…c64e5` for `{publish,delete,admin}`, committed in a public
   repo ⇒ everyone is admin. Remove; authority = git-push rights / signed tokens.
2. **Clerk JWT trusted with no signature check** — `verifyToken` base64-decodes
   `parts[1]` and trusts `sub`/`exp`, never verifying against `CLERK_SECRET_KEY`
   ⇒ a forged unsigned JWT yields `publish`. Verify properly or drop Clerk for
   git-credential auth.
3. **`handlePublish` persists nothing** — only `EDGE_PACKAGES.set()` (in-memory) +
   a `download_url` to a GitHub-raw tarball never uploaded ⇒ publishes vanish on
   eviction and 404 on download. Replace with decision 3 (`git commit`+`push`).
4. **`handleCreateToken` is ephemeral + identity-less** — hash stored in
   `EDGE_TOKENS` (lost on restart), `user_id` hardcoded `"clerk_publisher"`.
   Token records live in git/Clerk (decision 4).
5. **Dead `SUPABASE_*` vars** in `wrangler.toml` — the worker's `Env` declares no
   Supabase field at all (decision 5).

## Implementation update (2026-10-04)

- `wrangler.toml` now contains only public mirror config (`DOMAIN`,
  `REGISTRY_URL`, `STATIC_INDEX_URL`, `GITHUB_REPO`) plus optional
  `GITHUB_TOKEN` as a Worker secret for GitHub API rate limits. Clerk and
  Supabase config were removed.
- `registry-worker/index.ts` is now read-only. It serves `/health`, `/stats`,
  `/index.json`, `/search`, `/packages/:name`, `/download/:name/:version`, and
  `/blob/:sha256`; old write routes (`/tokens`, `/auth/create-token`,
  `/publish`) return `410 git_registry_only` with `keel publish` instructions.
- The website no longer depends on `@clerk/clerk-react` or `@clerk/themes`.
  The account surface is now a publisher guide explaining the git-backed
  workflow, current env, and exact Keel commands.
- `.github/workflows/pages.yml` regenerates the static registry index before
  building the website. `.github/workflows/registry-worker.yml` type-checks the
  Worker and deploys it when Cloudflare secrets are present.

## Principles
- **Decentralized, no single point of failure** — the registry is a git repo; every clone is a mirror.
- **No money, no card, no vendor lock-in** — any git remote works.
- **Offline-first** — `git clone` once → `keel fetch`/`add` need zero network after that.
- **Content-addressed + checksummed** — SHA-256 (`lpp-pm::ContentAddress`); tamper-evident, dedup by hash.
- **Fast** — an in-memory hot index over the local clone → instant, offline lookups.
- **Reproducible** — `Keel.lock` pins the exact resolved set + checksums.

## Repo layout (one git repo = the whole registry)
```
registry/
  index/
    <sparse-path for name>     → the per-package IndexEntry (JSON)
  blob/
    <sha256>                    → the package artifact (bytes), named by its content
```

**Sparse index path** (cargo-compatible, cache-friendly):
1 char → `1/<n>` · 2 → `2/<n>` · 3 → `3/<a>/<bc>` · 4+ → `<aa>/<bb>/<name>`

## How it works (git, not HTTP)
| Operation | What happens |
| --- | --- |
| **Fetch / add** (`keel fetch math`) | `git clone --depth 1 <registry>` (first time) or `git fetch` (update). Then read `index/…` **from disk** → get `blob/<sha256>` → read it **from disk** → verify SHA-256 → cache. No network after the clone. |
| **Search** (`keel search lin`) | Read every `index/…` doc from the local clone, match by name. Offline. |
| **Publish** (`keel publish`) | Write `blob/<sha256>` + the `index/…` doc, `git commit` (atomic), `git push`. One commit = index + artifact together. |

The local clone lives under the global cache: `~/.cache/keel/registry`.

## Package format
A package = `Keel.toml` + source, packed into an artifact (`.tar.gz`). The
artifact's SHA-256 is its content address, written into `index/…` and stored
at `blob/<sha256>`.

## Index schema (JSON — one file per package at `index/<sparse-path>`)
```json
{ "name": "mylib",
  "versions": [
    { "version": "1.2.0",
      "deps": [ { "name": "otherlib", "req": "^1.0", "optional": false, "features": [] } ],
      "features": { "default": [], "simd": [] },
      "checksum": "<sha256>",
      "targets": ["x86_64", "aarch64", "wasm32-wasi"],
      "yanked": false } ] }
```

## `Keel.toml` (the manifest)
```toml
[package]
name = "mylib"
version = "1.2.0"
edition = "2024"
license = "MIT OR Apache-2.0"

[dependencies]
otherlib = "1"                                          # version-only
simdlib  = { version = "2", optional = true, features = ["fast"] }  # detailed

[features]
default = []
simd = ["simdlib"]

[targets]
supported = ["x86_64", "aarch64", "wasm32-wasi"]
```

## Yanked and deprecated versions

- **Yanked** is the implemented enforcement mechanism today: each version row has
  `"yanked": false|true`. Keel resolution/update filters yanked versions, so new
  lockfile resolutions do not select them. Existing lockfiles remain verifiable by
  checksum so builds stay reproducible unless the blob is removed/tampered.
- **Deprecated** is not a separate schema field yet. If a version must no longer
  be selected, yank it. If it should remain installable but warn users, add a
  future additive metadata field such as `deprecated: "message"` and optional
  `replacement: "otherpkg"`; old Keel clients will ignore the unknown field while
  newer clients can show an advisory during `search`, `fetch`, `update`, and
  `outdated`.
- **Immutable publish still applies:** yanking/deprecation is index metadata in a
  new registry commit. A published `(package, version, checksum)` is never
  rewritten in place; duplicate version conflicts are refused.

## Security
- The artifact SHA-256 is published in the index; Keel **verifies it on every fetch** (tamper-evident). A blob whose hash doesn't match is refused (`E6009`).
- Publish = `git commit` + `git push` to a remote **you** control (a private repo, or a token); readers only need read access.
- History + provenance come free: every publish is a commit — the registry has a full, auditable history and can roll back.
- (Later) optional **signed packages** (signature over manifest + checksum) for a stronger trust model.

## How the hot index + content-addressing make it fast
- The local clone is the durable, offline **storage** layer.
- The **in-memory KV** is the hot index **on top of the clone** → repeated lookups are instant, no disk/network.
- **Delta** (slice 5): the resolver diffs index/lock → fetch only what's new (a single `git fetch`).
- **Offline**: warm clone ⇒ `keel build` with no network.

## Decentralization + the one trade-off
- **Mirrors**: clone the registry onto GitHub, a Gitea, a server, a laptop — all at once. Kill one, the rest survive.
- **Growth**: the repo grows as packages land. Mitigation, if it ever matters: `git clone --depth 1` (shallow) + `--filter=blob:none` (partial — download only the blobs you need). At a small scale a full clone is tiny.

## PM re-architecture (`lpp-pm` modules)
| Module | Role | Status |
| --- | --- | --- |
| `address` | SHA-256 content addressing | ✅ |
| `store` | durable content-addressed blob store | ✅ |
| `cache` | hot KV cache (in-memory) | ✅ |
| `manifest` | `Keel.toml` model + parse | ✅ |
| `index` | index schema + sparse path layout | ✅ |
| `registry` | git-backed client: sync (clone/fetch) · lookup · fetch+verify · list/search · publish · push | ✅ |
| `lock` | `Keel.lock` resolved-set model | next |
| `resolve` | the dependency resolver | next |

`keel` (the CLI) drives the registry client: `keel fetch <name>[@version]`,
`keel search <query>`, `keel publish`. The registry URL comes from
`--registry <git-url>` or the `KEEL_REGISTRY` env var.
