# L++ Package Registry API

Cloudflare Worker backend for the L++ package registry at `https://registry.lplusplus.bond`.

## Architecture

Same pattern as Cloud Foundation Hub:
- **Cloudflare Workers** — serves the API at `registry.lplusplus.bond`
- **Supabase** — PostgreSQL backend with:
  - `api_keys` table — scoped API keys with rate limits
  - `packages` table — JSONB metadata for each package
  - `request_logs` table — audit trail + usage analytics
- **Cloudflare DNS** — `lplusplus.bond` → Pages, `registry.lplusplus.bond` → Workers

## Setup

### 1. Create Supabase Project
```bash
# Option A: Use existing Supabase project
# Update SUPABASE_URL and SUPABASE_SERVICE_ROLE_KEY in wrangler.toml env vars

# Option B: Create new project
supabase login
supabase projects create lplusplus-registry
supabase link --project-ref <your-project-ref>
supabase db push
```

### 2. Deploy Worker
```bash
wrangler login
wrangler deploy
```

### 3. Configure DNS in Cloudflare Dashboard
```
# Root domain
lplusplus.bond       → CNAME → <pages-project>.pages.dev
registry.lplusplus.bond → CNAME → <worker>.lplusplus.workers.dev
```

## API Endpoints

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/` | None | Full registry manifest |
| GET | `/index.json` | None | Same as above |
| GET | `/names/:shard` | None | Package name shard |
| GET | `/search?q=term` | None | Search packages |
| GET | `/packages/:name` | None | Get package metadata |
| POST | `/publish` | `api:write` scope | Publish a package |
| GET | `/health` | None | Health check |

## Publish Command

From an L++ package directory:
```bash
export LPP_REGISTRY_URL=https://registry.lplusplus.bond
export LPP_API_KEY=sk_layer_yourkey

# Publish with auto-bump
lpp publish --bump patch

# Dry run
lpp publish --dry-run

# Force specific version
lpp publish --version 1.2.3
```

## Environment Variables

Set these in `wrangler.toml` or Cloudflare dashboard:

| Variable | Required | Description |
|----------|----------|-------------|
| `SUPABASE_URL` | Yes | Supabase project URL |
| `SUPABASE_SERVICE_ROLE_KEY` | Yes | Service role key (bypasses RLS) |
| `DOMAIN` | No | Domain name (default: lplusplus.bond) |
| `REGISTRY_URL` | No | Registry base URL |

## Local Development

```bash
# Install dependencies
cd registry-worker
npm install

# Run dev server with Supabase emulator
wrangler dev
```

## Migration from GitHub Pages Registry

The old registry was served from `samarnever-droid.github.io/lplusplus/registry/index.json`.

Update consumers to use:
```bash
export LPP_REGISTRY_URL=https://registry.lplusplus.bond
```

Or update `src/pm.rs` default (already done in this repo).
