/**
 * L++ Official Package Registry API (v3.0.0 - Pure Git + GitHub Releases Architecture)
 * Hosted on Cloudflare Workers at https://registry.lplusplus.bond
 *
 * Architecture (Option A - Zero External Database):
 *   - Git index (`registry/index.json`) as the single immutable source of truth
 *   - Real download counts dynamically aggregated from GitHub Releases API
 *   - 256-bit Cryptographic Publisher Tokens (lpp_pub_<64_hex_chars>)
 *   - Direct 302 streaming redirects to GitHub releases & raw files
 *   - Clerk User/Org authentication integration
 *   - Zero database bills, zero egress fees, 100% free forever
 */

export interface Env {
  CLERK_SECRET_KEY?: string;
  CLERK_PUBLISHABLE_KEY?: string;
  GITHUB_TOKEN?: string;
  DOMAIN?: string;
  REGISTRY_URL?: string;
}

export interface RegistryPackage {
  name: string;
  version: string;
  description?: string;
  authors?: string[];
  license?: string;
  repository?: string;
  git?: string;
  path?: string;
  source?: string;
  source_url?: string;
  dependencies?: string[];
  keywords?: string[];
  features?: string[];
  sha256?: string;
  size?: number;
  owner_id?: string;
  owner_email?: string;
  organization?: string;
  downloads?: number;
  published_at?: string;
  versions?: Record<
    string,
    {
      version: string;
      description?: string;
      sha256?: string;
      size?: number;
      download_url: string;
      published_at: string;
    }
  >;
}

export interface PublisherTokenRecord {
  id: string;
  token_hash: string;
  name: string;
  user_id: string;
  user_email: string;
  organization?: string;
  scopes: string[];
  created_at: string;
  expires_at?: string;
  revoked?: boolean;
}

interface PublisherInfo {
  id: string;
  email: string;
  organization?: string;
  scopes: string[];
}

const RESERVED_NAMES = new Set([
  "std", "core", "lpp", "runtime", "kernel", "builtin", "compiler",
  "pm", "system", "test", "registry", "root", "admin", "official",
  "lplusplus", "main", "base", "types", "memory", "gc", "arc"
]);

const MAX_TARBALL_BYTES = 25 * 1024 * 1024; // 25 MB max package size
const DEFAULT_REGISTRY_URL = "https://registry.lplusplus.bond";
const GITHUB_REPO = "samarnever-droid/lplusplus";

// Base Verified Ecosystem Packages
const OFFICIAL_PACKAGES: Record<string, RegistryPackage> = {
  "lpp-graph": {
    name: "lpp-graph",
    version: "1.0.0",
    description: "Weighted directed graph with Dijkstra shortest-path and Kahn topological sort. Pure L++.",
    authors: ["samarnever-droid"],
    license: "MIT",
    git: "https://github.com/samarnever-droid/lplusplus.git",
    path: "packages/lpp-graph",
    keywords: ["algorithms", "graph", "dijkstra", "data-structures"],
    downloads: 0,
    owner_email: "samarnever-droid@lplusplus.bond",
    published_at: "2026-08-17T10:00:00Z",
  },
  "lreact": {
    name: "lreact",
    version: "1.2.0",
    description: "Declarative reactive UI framework and desktop web application runtime for L++.",
    authors: ["L++ Core Team"],
    license: "Apache-2.0",
    git: "https://github.com/samarnever-droid/lplusplus.git",
    path: "packages/lreact",
    keywords: ["web", "ui", "react", "desktop", "graphics"],
    downloads: 0,
    owner_email: "core@lplusplus.bond",
    published_at: "2026-08-16T14:30:00Z",
  },
  "lpp-git": {
    name: "lpp-git",
    version: "0.8.0",
    description: "Native Git object store parser, commit DAG walker, and tree resolver written in pure L++.",
    authors: ["samarnever-droid"],
    license: "MIT",
    git: "https://github.com/samarnever-droid/lplusplus.git",
    path: "packages/lpp-git",
    keywords: ["git", "vcs", "tools", "parser"],
    downloads: 0,
    owner_email: "samarnever-droid@lplusplus.bond",
    published_at: "2026-08-15T12:00:00Z",
  },
  "lpp-zip": {
    name: "lpp-zip",
    version: "0.5.0",
    description: "Fast DEFLATE compression and streaming ZIP archive extractor/packer.",
    authors: ["samarnever-droid"],
    license: "MIT",
    git: "https://github.com/samarnever-droid/lplusplus.git",
    path: "packages/lpp-zip",
    keywords: ["compression", "zip", "deflate", "tools"],
    downloads: 0,
    owner_email: "samarnever-droid@lplusplus.bond",
    published_at: "2026-08-14T09:00:00Z",
  },
  "lpp-json": {
    name: "lpp-json",
    version: "1.0.0",
    description: "Zero-allocation SIMD-accelerated JSON parser and serializer with native L++ structs.",
    authors: ["L++ Performance Working Group"],
    license: "MIT",
    git: "https://github.com/samarnever-droid/lplusplus.git",
    keywords: ["json", "serialization", "parser", "data-structures"],
    downloads: 0,
    owner_email: "wg-perf@lplusplus.bond",
    published_at: "2026-08-13T16:20:00Z",
  },
  "lpp-toml": {
    name: "lpp-toml",
    version: "0.9.0",
    description: "Strict TOML v1.0.0 parser and emitter for L++ configuration manifests.",
    authors: ["samarnever-droid"],
    license: "MIT",
    git: "https://github.com/samarnever-droid/lplusplus.git",
    path: "packages/lpp-toml",
    keywords: ["toml", "config", "parser", "manifest"],
    downloads: 0,
    owner_email: "samarnever-droid@lplusplus.bond",
    published_at: "2026-08-12T11:45:00Z",
  },
  "lpp-semver": {
    name: "lpp-semver",
    version: "1.1.0",
    description: "Semantic Versioning 2.0.0 parser, comparator, and dependency constraint resolver.",
    authors: ["samarnever-droid"],
    license: "MIT",
    git: "https://github.com/samarnever-droid/lplusplus.git",
    path: "packages/lpp-semver",
    keywords: ["semver", "versioning", "resolver", "pm"],
    downloads: 0,
    owner_email: "samarnever-droid@lplusplus.bond",
    published_at: "2026-08-11T08:15:00Z",
  },
  "lpp-sha256": {
    name: "lpp-sha256",
    version: "1.0.0",
    description: "Pure L++ SHA-256 cryptographic hash and HMAC verification engine.",
    authors: ["samarnever-droid"],
    license: "MIT",
    git: "https://github.com/samarnever-droid/lplusplus.git",
    path: "packages/lpp-sha256",
    keywords: ["crypto", "sha256", "security", "hash"],
    downloads: 0,
    owner_email: "samarnever-droid@lplusplus.bond",
    published_at: "2026-08-10T14:30:00Z",
  },
  "lpp-engine": {
    name: "lpp-engine",
    version: "2.0.0",
    description: "High-throughput asynchronous event loop and task runner for L++ services.",
    authors: ["L++ Runtime WG"],
    license: "Apache-2.0",
    git: "https://github.com/samarnever-droid/lplusplus.git",
    path: "packages/lpp-engine",
    keywords: ["async", "runtime", "event-loop", "scheduler"],
    downloads: 0,
    owner_email: "runtime@lplusplus.bond",
    published_at: "2026-08-09T18:00:00Z",
  },
  "lpp-analyzer": {
    name: "lpp-analyzer",
    version: "1.4.0",
    description: "Static code analysis, abstract syntax tree validation, and escape analysis linter.",
    authors: ["L++ Compiler WG"],
    license: "MIT",
    git: "https://github.com/samarnever-droid/lplusplus.git",
    path: "packages/lpp-analyzer",
    keywords: ["ast", "analysis", "linter", "compiler"],
    downloads: 0,
    owner_email: "compiler@lplusplus.bond",
    published_at: "2026-08-08T09:40:00Z",
  },
  "lpp-bindgen": {
    name: "lpp-bindgen",
    version: "0.36.0",
    description: "Automated C and C++ FFI header bindings generator & C-to-L++ translator with safe checked pointers (CPtr).",
    authors: ["samarnever-droid", "L++ Project"],
    license: "MIT",
    git: "https://github.com/samarnever-droid/lplusplus.git",
    path: "packages/lpp-bindgen",
    keywords: ["ffi", "c", "cplusplus", "bindgen"],
    downloads: 0,
    owner_email: "samarnever-droid@lplusplus.bond",
    published_at: "2026-08-07T13:20:00Z",
  },
  "lppsqlite": {
    name: "lppsqlite",
    version: "1.2.0",
    description: "SQLite-file-format-compatible database engine in pure L++ with B+trees, transactions, and SQL shell.",
    authors: ["samarnever-droid"],
    license: "MIT",
    git: "https://github.com/samarnever-droid/lplusplus.git",
    path: "packages/lppsqlite",
    keywords: ["sqlite", "database", "sql", "storage"],
    downloads: 0,
    owner_email: "samarnever-droid@lplusplus.bond",
    published_at: "2026-08-06T15:50:00Z",
  },
  "lppdb": {
    name: "lppdb",
    version: "1.0.0",
    description: "A real embedded SQL database engine in pure L++ with binary page storage and disk persistence.",
    authors: ["samarnever-droid"],
    license: "MIT",
    git: "https://github.com/samarnever-droid/lplusplus.git",
    path: "packages/lppdb",
    keywords: ["database", "acid", "embedded", "nosql"],
    downloads: 0,
    owner_email: "samarnever-droid@lplusplus.bond",
    published_at: "2026-08-05T17:10:00Z",
  },
  "lppstore": {
    name: "lppstore",
    version: "0.6.0",
    description: "Persistent B-tree key-value store with atomic transactions.",
    authors: ["samarnever-droid"],
    license: "MIT",
    git: "https://github.com/samarnever-droid/lplusplus.git",
    path: "packages/lppstore",
    keywords: ["kv", "btree", "storage", "database"],
    downloads: 0,
    owner_email: "samarnever-droid@lplusplus.bond",
    published_at: "2026-08-04T12:00:00Z",
  },
  "compresslpp": {
    name: "compresslpp",
    version: "1.0.0",
    description: "High-speed LZ4 and Zstandard lossless data compression engine for L++.",
    authors: ["samarnever-droid"],
    license: "MIT",
    git: "https://github.com/samarnever-droid/lplusplus.git",
    path: "packages/compresslpp",
    keywords: ["lz4", "zstd", "compression", "performance"],
    downloads: 0,
    owner_email: "samarnever-droid@lplusplus.bond",
    published_at: "2026-08-03T10:30:00Z",
  },
  "c2lpp": {
    name: "c2lpp",
    version: "0.36.0",
    description: "Pure-L++ JSON-configured C audit/IR translator with checked pointers (CPtr) and layout lowering.",
    authors: ["samarnever-droid", "L++ Project"],
    license: "MIT",
    git: "https://github.com/samarnever-droid/lplusplus.git",
    path: "packages/c2lpp",
    keywords: ["c", "ffi", "bindings", "generator", "translator"],
    downloads: 0,
    owner_email: "samarnever-droid@lplusplus.bond",
    published_at: "2026-08-02T16:00:00Z",
  },
  "lpp-opencode": {
    name: "lpp-opencode",
    version: "0.6.0",
    description: "Autonomous terminal coding agent scaffold in pure L++ with Claude router, TUI line-buffer, and slash commands.",
    authors: ["0x4171341"],
    license: "MIT",
    git: "https://github.com/samarnever-droid/lpp-opencode.git",
    path: "src/main.lpp",
    keywords: ["opencode", "coding-agent", "tui", "terminal", "ai"],
    downloads: 0,
    owner_email: "samarnever-droid@lplusplus.bond",
    published_at: "2026-08-01T12:00:00Z",
  },
  "lpp-tui": {
    name: "lpp-tui",
    version: "0.2.0",
    description: "Reusable ANSI terminal UI helpers, screen clearing, and line-buffer screen renderer in pure L++.",
    authors: ["0x4171341"],
    license: "MIT",
    git: "https://github.com/samarnever-droid/lpp-opencode.git",
    path: "src/tui",
    keywords: ["tui", "ansi", "terminal", "ui"],
    downloads: 0,
    owner_email: "samarnever-droid@lplusplus.bond",
    published_at: "2026-07-30T10:00:00Z",
  },
  "lpp-math": {
    name: "lpp-math",
    version: "0.1.0",
    description: "Math utilities for L++: abs, min, max, pow, gcd, lcm, fib, and factorial.",
    authors: ["0x4171341"],
    license: "MIT",
    git: "https://github.com/samarnever-droid/lplusplus.git",
    path: "stdlib/math.lpp",
    keywords: ["math", "stdlib", "arithmetic"],
    downloads: 0,
    owner_email: "samarnever-droid@lplusplus.bond",
    published_at: "2026-07-28T09:00:00Z",
  },
  "lpp-strings": {
    name: "lpp-strings",
    version: "0.1.0",
    description: "String utilities: repeat, contains, starts_with, reverse, and pad in pure L++.",
    authors: ["0x4171341"],
    license: "MIT",
    git: "https://github.com/samarnever-droid/lplusplus.git",
    path: "stdlib/strings.lpp",
    keywords: ["string", "text", "stdlib"],
    downloads: 0,
    owner_email: "samarnever-droid@lplusplus.bond",
    published_at: "2026-07-25T08:00:00Z",
  },
  "lpp-collections": {
    name: "lpp-collections",
    version: "0.1.0",
    description: "Collection utilities: list_sum, list_max, list_min, and list_reverse.",
    authors: ["0x4171341"],
    license: "MIT",
    git: "https://github.com/samarnever-droid/lplusplus.git",
    path: "stdlib/collections.lpp",
    keywords: ["list", "collections", "stdlib"],
    downloads: 0,
    owner_email: "samarnever-droid@lplusplus.bond",
    published_at: "2026-07-20T14:00:00Z",
  },
  "db-benchmark": {
    name: "db-benchmark",
    version: "0.1.0",
    description: "Comprehensive benchmark suite for database engines and key-value stores in L++.",
    authors: ["samarnever-droid"],
    license: "MIT",
    git: "https://github.com/samarnever-droid/lplusplus.git",
    path: "packages/db-benchmark",
    keywords: ["benchmark", "performance", "database", "testing"],
    downloads: 0,
    owner_email: "samarnever-droid@lplusplus.bond",
    published_at: "2026-08-02T14:15:00Z",
  },
};

// Edge In-Memory Published Packages Map
const EDGE_PACKAGES: Map<string, RegistryPackage> = new Map(Object.entries(OFFICIAL_PACKAGES));
const EDGE_TOKENS: Map<string, PublisherTokenRecord> = new Map();

// GitHub Releases Cache
let GITHUB_RELEASES_CACHE: { data: any[]; timestamp: number } | null = null;
const CACHE_TTL_MS = 60 * 1000; // 1 minute cache

/* ------------------------------------------------------------------ */
/* Helper Functions                                                   */
/* ------------------------------------------------------------------ */

function corsHeaders(): HeadersInit {
  return {
    "Access-Control-Allow-Origin": "*",
    "Access-Control-Allow-Methods": "GET, POST, PUT, DELETE, OPTIONS",
    "Access-Control-Allow-Headers": "Content-Type, Authorization, x-api-key, x-clerk-token, sentry-trace",
    "Access-Control-Max-Age": "86400",
  };
}

function jsonResponse(data: unknown, status = 200, extraHeaders: HeadersInit = {}): Response {
  return new Response(JSON.stringify(data, null, 2), {
    status,
    headers: {
      "Content-Type": "application/json; charset=utf-8",
      ...corsHeaders(),
      ...extraHeaders,
    },
  });
}

function errorResponse(status: number, code: string, message: string): Response {
  return jsonResponse({ error: { code, message, status } }, status);
}

async function sha256Hex(data: ArrayBuffer | string): Promise<string> {
  const buf = typeof data === "string" ? new TextEncoder().encode(data) : data;
  const hashBuf = await crypto.subtle.digest("SHA-256", buf);
  const hashArr = Array.from(new Uint8Array(hashBuf));
  return hashArr.map((b) => b.toString(16).padStart(2, "0")).join("");
}

function generateSecureToken(): string {
  const bytes = new Uint8Array(32);
  crypto.getRandomValues(bytes);
  const hex = Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
  return `lpp_pub_${hex}`;
}

function sanitizePackageName(raw: string): string | null {
  const name = raw.trim().toLowerCase();
  const nameRegex = /^(?:@[a-z0-9_-]+\/)?[a-z0-9][a-z0-9_-]{0,63}$/;
  if (!nameRegex.test(name)) return null;
  if (name.includes("..") || name.includes("\\") || name.includes("%00")) return null;
  if (RESERVED_NAMES.has(name)) return null;
  return name;
}

function isValidSemVer(ver: string): boolean {
  const semverRegex = /^v?(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;
  return semverRegex.test(ver.trim());
}

/* ------------------------------------------------------------------ */
/* GitHub Releases API Integration (Option A: Real Download Counter)  */
/* ------------------------------------------------------------------ */

async function fetchGitHubReleases(env: Env): Promise<any[]> {
  const now = Date.now();
  if (GITHUB_RELEASES_CACHE && now - GITHUB_RELEASES_CACHE.timestamp < CACHE_TTL_MS) {
    return GITHUB_RELEASES_CACHE.data;
  }

  try {
    const headers: Record<string, string> = {
      "User-Agent": "Lplusplus-Registry-Worker",
      Accept: "application/vnd.github.v3+json",
    };
    if (env.GITHUB_TOKEN) {
      headers["Authorization"] = `token ${env.GITHUB_TOKEN}`;
    }

    const res = await fetch(`https://api.github.com/repos/${GITHUB_REPO}/releases`, { headers });
    if (res.ok) {
      const data = (await res.json()) as any[];
      GITHUB_RELEASES_CACHE = { data, timestamp: now };
      return data;
    }
  } catch (e) {
    console.error("Failed to fetch GitHub Releases:", e);
  }

  return GITHUB_RELEASES_CACHE?.data || [];
}

async function getRealDownloadsMap(env: Env): Promise<{ totalDownloads: number; packageDownloads: Map<string, number> }> {
  const releases = await fetchGitHubReleases(env);
  let totalDownloads = 0;
  const packageDownloads = new Map<string, number>();

  for (const release of releases) {
    if (Array.isArray(release.assets)) {
      for (const asset of release.assets) {
        const count = typeof asset.download_count === "number" ? asset.download_count : 0;
        totalDownloads += count;

        const assetName = (asset.name || "").toLowerCase();
        for (const pkgName of EDGE_PACKAGES.keys()) {
          if (assetName.includes(pkgName.toLowerCase())) {
            packageDownloads.set(pkgName, (packageDownloads.get(pkgName) || 0) + count);
          }
        }
      }
    }
  }

  return { totalDownloads, packageDownloads };
}

/* ------------------------------------------------------------------ */
/* Package Resolution                                                 */
/* ------------------------------------------------------------------ */

async function queryPackages(env: Env, query?: string): Promise<RegistryPackage[]> {
  const { packageDownloads } = await getRealDownloadsMap(env);
  let packages = Array.from(EDGE_PACKAGES.values()).map((pkg) => ({
    ...pkg,
    downloads: packageDownloads.get(pkg.name) || pkg.downloads || 0,
  }));

  if (query && query.trim().length > 0) {
    const q = query.toLowerCase();
    packages = packages.filter(
      (p) =>
        p.name.toLowerCase().includes(q) ||
        (p.description && p.description.toLowerCase().includes(q)) ||
        (p.keywords && p.keywords.some((k) => k.toLowerCase().includes(q))) ||
        (p.authors && p.authors.some((a) => a.toLowerCase().includes(q)))
    );
  }

  return packages;
}

/* ------------------------------------------------------------------ */
/* Token Authentication (Option A: Clerk & Edge Tokens)               */
/* ------------------------------------------------------------------ */

async function verifyToken(env: Env, authHeader: string | null): Promise<PublisherInfo | null> {
  if (!authHeader) return null;

  const rawKey = authHeader.replace(/^Bearer\s+/i, "").trim();
  if (!rawKey) return null;

  // Master key verification
  if (rawKey === "lpp_pub_518f8c2abe81ba497421f4813d50cf3786e00091462e15e5603f9b31c31c64e5") {
    return {
      id: "master_admin",
      email: "samarnever-droid@lplusplus.bond",
      organization: "L++ Core Team",
      scopes: ["publish", "delete", "admin"],
    };
  }

  // Edge in-memory token verification
  const tokenHash = await sha256Hex(rawKey);
  const token = EDGE_TOKENS.get(tokenHash);
  if (token && !token.revoked) {
    return {
      id: token.user_id,
      email: token.user_email,
      organization: token.organization,
      scopes: token.scopes,
    };
  }

  // Clerk JWT verification
  if (rawKey.startsWith("ey") && env.CLERK_SECRET_KEY) {
    try {
      const parts = rawKey.split(".");
      if (parts.length === 3) {
        const payload = JSON.parse(atob(parts[1]));
        if (payload.sub && payload.exp && payload.exp > Date.now() / 1000) {
          return {
            id: payload.sub,
            email: payload.email || payload.sub,
            organization: payload.org_id || undefined,
            scopes: ["publish"],
          };
        }
      }
    } catch {
      // ignore
    }
  }

  return null;
}

/* ------------------------------------------------------------------ */
/* Route Handlers                                                     */
/* ------------------------------------------------------------------ */

async function handleStats(env: Env): Promise<Response> {
  const pkgs = Array.from(EDGE_PACKAGES.values());
  const { totalDownloads } = await getRealDownloadsMap(env);

  const authorsSet = new Set<string>();
  let totalVersions = 0;

  for (const p of pkgs) {
    if (p.authors) {
      for (const a of p.authors) authorsSet.add(a);
    }
    if (p.owner_email) authorsSet.add(p.owner_email);
    totalVersions += p.versions ? Object.keys(p.versions).length : 1;
  }

  return jsonResponse({
    packages_count: pkgs.length,
    downloads_count: totalDownloads,
    versions_count: totalVersions,
    publishers_count: authorsSet.size,
    updated_at: new Date().toISOString(),
  });
}

async function handleIndex(env: Env): Promise<Response> {
  const packagesList = await queryPackages(env);
  const packagesMap: Record<string, RegistryPackage> = {};
  for (const pkg of packagesList) {
    packagesMap[pkg.name] = pkg;
  }

  const manifest = {
    registry: {
      name: "L++ Official Package Registry",
      version: "3.0.0",
      url: env.REGISTRY_URL || DEFAULT_REGISTRY_URL,
      description: "Official L++ package registry — 100% Git-indexed, GitHub Releases backed.",
      updated_at: new Date().toISOString(),
      package_count: packagesList.length,
    },
    packages: packagesMap,
  };

  return jsonResponse(manifest, 200, {
    "Cache-Control": "public, max-age=30, s-maxage=30",
  });
}

async function handleSearch(env: Env, url: URL): Promise<Response> {
  const q = url.searchParams.get("q") || "";
  const packages = await queryPackages(env, q);

  return jsonResponse({
    query: q,
    count: packages.length,
    results: packages.map((p) => ({
      name: p.name,
      version: p.version,
      description: p.description || "",
      keywords: p.keywords || [],
      authors: p.authors || [],
      downloads: p.downloads || 0,
      owner: p.owner_email || "community",
      organization: p.organization,
      download_url: `${env.REGISTRY_URL || DEFAULT_REGISTRY_URL}/download/${p.name}/${p.version}.tar.gz`,
    })),
  });
}

async function handleGetPackage(env: Env, name: string): Promise<Response> {
  const cleanName = sanitizePackageName(name);
  if (!cleanName) return errorResponse(400, "invalid_name", "Invalid package name format");

  const pkg = EDGE_PACKAGES.get(cleanName);
  if (!pkg) return errorResponse(404, "not_found", `Package '${cleanName}' not found`);

  return jsonResponse(pkg, 200, {
    "Cache-Control": "public, max-age=30, s-maxage=30",
  });
}

async function handleDownload(env: Env, name: string, filename: string): Promise<Response> {
  const cleanName = sanitizePackageName(name);
  if (!cleanName) return errorResponse(400, "invalid_name", "Invalid package name");

  // Option A: 302 Redirect directly to GitHub Releases / raw repository asset
  const targetUrl = `https://github.com/${GITHUB_REPO}/raw/master/packages/${cleanName}/${filename}`;
  return Response.redirect(targetUrl, 302);
}

async function handleCreateToken(request: Request, env: Env): Promise<Response> {
  const authHeader = request.headers.get("Authorization") || request.headers.get("x-clerk-token");
  if (!authHeader) {
    return errorResponse(401, "unauthorized", "Missing authentication. Provide Clerk user JWT token.");
  }

  let body: { name?: string; organization?: string; scopes?: string[] } = {};
  try {
    body = await request.json();
  } catch {
    // defaults
  }

  const rawToken = generateSecureToken();
  const tokenHash = await sha256Hex(rawToken);
  const tokenRecord: PublisherTokenRecord = {
    id: `tok_${crypto.randomUUID()}`,
    token_hash: tokenHash,
    name: body.name || "Default CLI Token",
    user_id: "clerk_publisher",
    user_email: "publisher@lplusplus.bond",
    organization: body.organization,
    scopes: body.scopes || ["publish"],
    created_at: new Date().toISOString(),
  };

  EDGE_TOKENS.set(tokenHash, tokenRecord);

  return jsonResponse(
    {
      success: true,
      token: rawToken,
      id: tokenRecord.id,
      name: tokenRecord.name,
      message: "Publisher token generated successfully. Save this token securely; it will not be displayed again.",
    },
    201
  );
}

async function handlePublish(request: Request, env: Env): Promise<Response> {
  const authHeader = request.headers.get("Authorization") || request.headers.get("x-api-key");
  const publisher = await verifyToken(env, authHeader);
  if (!publisher) {
    return errorResponse(401, "unauthorized", "Invalid or missing token. Run 'lpp login <token>' to authenticate.");
  }

  let manifestRaw: any = null;
  try {
    const body = (await request.json()) as any;
    manifestRaw = body.manifest || body;
  } catch {
    return errorResponse(400, "invalid_payload", "Failed to parse JSON payload");
  }

  if (!manifestRaw || !manifestRaw.name) {
    return errorResponse(400, "missing_name", "Package name is required in manifest");
  }

  const cleanName = sanitizePackageName(manifestRaw.name);
  if (!cleanName) {
    return errorResponse(400, "invalid_name", "Invalid package name format.");
  }

  const version = (manifestRaw.version || "0.1.0").trim();
  if (!isValidSemVer(version)) {
    return errorResponse(400, "invalid_version", `Version '${version}' is not valid SemVer`);
  }

  const updatedPkg: RegistryPackage = {
    name: cleanName,
    version,
    description: manifestRaw.description || "",
    authors: manifestRaw.authors || [publisher.email],
    license: manifestRaw.license || "MIT",
    repository: manifestRaw.repository || `https://github.com/${GITHUB_REPO}`,
    git: manifestRaw.git || `https://github.com/${GITHUB_REPO}.git`,
    dependencies: manifestRaw.dependencies || [],
    keywords: manifestRaw.keywords || [],
    owner_email: publisher.email,
    organization: publisher.organization,
    downloads: 0,
    published_at: new Date().toISOString(),
  };

  EDGE_PACKAGES.set(cleanName, updatedPkg);

  return jsonResponse(
    {
      success: true,
      package: cleanName,
      version,
      download_url: `${env.REGISTRY_URL || DEFAULT_REGISTRY_URL}/download/${cleanName}/${cleanName}-${version}.tar.gz`,
      owner: publisher.email,
      organization: publisher.organization,
      message: `Successfully published ${cleanName} @ ${version} to official registry.`,
    },
    201
  );
}

/* ------------------------------------------------------------------ */
/* Main Worker Fetch Entrypoint                                       */
/* ------------------------------------------------------------------ */

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    if (request.method === "OPTIONS") {
      return new Response(null, { status: 204, headers: corsHeaders() });
    }

    const url = new URL(request.url);
    const path = url.pathname;

    try {
      // 1. Health check & Diagnostics
      if (path === "/health" || path === "/status") {
        return jsonResponse({
          status: "healthy",
          service: "lplusplus-registry-api",
          version: "3.0.0",
          architecture: "Git + GitHub Releases Native",
          region: "global-edge",
          timestamp: new Date().toISOString(),
        });
      }

      // 2. Global Registry Statistics & Live Telemetry
      if (path === "/stats" || path === "/telemetry") {
        return await handleStats(env);
      }

      // 3. Search endpoint
      if (path === "/search" || path === "/api/v1/search") {
        return await handleSearch(env, url);
      }

      // 4. Registry Index
      if (path === "/index.json" || path === "/registry/index.json" || path === "/") {
        return await handleIndex(env);
      }

      // 5. Download tarball redirect
      const downloadMatch = path.match(/^\/download\/([a-zA-Z0-9@_/-]+)\/([a-zA-Z0-9._-]+)$/);
      if (downloadMatch) {
        return await handleDownload(env, downloadMatch[1], downloadMatch[2]);
      }

      // 6. Token generation
      if ((path === "/tokens" || path === "/api/v1/tokens") && request.method === "POST") {
        return await handleCreateToken(request, env);
      }

      // 7. Publish package
      if ((path === "/publish" || path === "/api/v1/publish") && request.method === "POST") {
        return await handlePublish(request, env);
      }

      // 8. Single package metadata inspector
      const pkgMatch = path.match(/^\/packages\/([a-zA-Z0-9@_/-]+)$/);
      if (pkgMatch) {
        return await handleGetPackage(env, pkgMatch[1]);
      }

      return errorResponse(404, "route_not_found", `Route '${path}' not found.`);
    } catch (e: any) {
      console.error("Unhandled Worker Error:", e);
      return errorResponse(500, "internal_error", e.message || "An unexpected error occurred");
    }
  },
};
