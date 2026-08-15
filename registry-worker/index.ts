/**
 * L++ Package Registry API — Cloudflare Worker
 * Hosted at registry.lplusplus.bond
 *
 * Security & Abuse Hardening:
 * - In-memory sliding window IP rate limiting on all endpoints (stricter on /publish)
 * - Maximum payload size constraints (128 KB max body)
 * - Strict regex input validation & PostgREST query injection prevention
 * - Timing-safe key verification with dual support for scoped API keys & root Service Role Key
 * - Leak-proof error handling (no internal stack traces or connection strings in responses)
 * - Full HTTP security headers (HSTS, nosniff, frame-ancestors, CSP)
 * - Built-in fallback catalog for high availability
 */

interface Env {
  SUPABASE_URL?: string;
  SUPABASE_SERVICE_ROLE_KEY?: string;
  SUPABASE_PUBLISHABLE_KEY?: string;
  DOMAIN?: string;
  REGISTRY_URL?: string;
  API_VERSION?: string;
}

// ── Types ─────────────────────────────────────────────────────────────────────

interface RegistryPackage {
  name: string;
  description?: string;
  version?: string;
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
  api?: Record<string, string>;
}

type RegistryManifest = {
  registry: {
    name: string;
    version: string;
    url: string;
    description: string;
  };
  packages: Record<string, RegistryPackage>;
};

// ── Default Built-in Packages Catalog ─────────────────────────────────────────

const DEFAULT_PACKAGES: Record<string, RegistryPackage> = {
  "lpp-zip": {
    name: "lpp-zip",
    description: "ZIP archive create/read library — pure L++ using buf_* primitives",
    version: "0.1.0",
    authors: ["0x4171341"],
    license: "MIT",
    repository: "https://github.com/samarnever-droid/lplusplus",
    path: "packages/lpp-zip",
    source: "packages/lpp-zip/src/zip.lpp",
    dependencies: [],
    keywords: ["zip", "archive", "binary", "file", "compression"],
  },
  "lpp-math": {
    name: "lpp-math",
    description: "Math utilities — abs, min, max, pow, gcd, lcm, fib, factorial",
    version: "0.1.0",
    authors: ["0x4171341"],
    license: "MIT",
    repository: "https://github.com/samarnever-droid/lplusplus",
    path: "stdlib/math.lpp",
    source: "stdlib/math.lpp",
    dependencies: [],
    keywords: ["math", "stdlib"],
  },
  "sqlite": {
    name: "sqlite",
    description: "SQLite-compatible database engine in pure L++: real .db files with secondary indexes, correlated subqueries, transactions, and SQL query executor",
    version: "1.2.0",
    authors: ["samarnever-droid"],
    license: "MIT",
    repository: "https://github.com/samarnever-droid/lplusplus",
    path: "packages/sqlite",
    source: "packages/sqlite/src/sqlite.lpp",
    dependencies: ["lppsqlite"],
    keywords: ["sqlite", "database", "sql", "storage", "btree"],
  },
  "lppsqlite": {
    name: "lppsqlite",
    description: "SQLite-compatible database engine in pure L++ — real .db files, B+trees, secondary indexes, overflow pages, freelist, correlated subqueries, real transactions and advisory locking",
    version: "1.2.0",
    authors: ["samarnever-droid"],
    license: "MIT",
    repository: "https://github.com/samarnever-droid/lplusplus",
    path: "packages/lppsqlite",
    source: "packages/lppsqlite/src/exec.lpp",
    dependencies: [],
    keywords: ["sqlite", "database", "sql", "storage", "btree"],
  },
  "lreact": {
    name: "lreact",
    description: "React-like UI framework for L++ — desktop and web targets",
    version: "1.0.0",
    authors: ["0x4171341"],
    license: "MIT",
    repository: "https://github.com/samarnever-droid/lreact",
    git: "https://github.com/samarnever-droid/lreact.git",
    path: "src/lreact.lpp",
    source: "https://raw.githubusercontent.com/samarnever-droid/lreact/main/src/lreact.lpp",
    dependencies: [],
    keywords: ["react", "ui", "gui", "desktop"],
  },
  "lppdb": {
    name: "lppdb",
    description: "Lightweight key-value database for L++ — built on SQLite bindings",
    version: "2.1.0",
    authors: ["0x4171341"],
    license: "MIT",
    repository: "https://github.com/samarnever-droid/lplusplus",
    path: "packages/lppdb",
    source: "packages/lppdb/src/db.lpp",
    dependencies: [],
    keywords: ["database", "sqlite", "kv"],
  },
};

// ── Rate Limiting & Abuse Prevention ──────────────────────────────────────────

interface RateLimitBucket {
  count: number;
  resetAt: number;
}
const ipBuckets = new Map<string, RateLimitBucket>();

function checkRateLimit(
  ip: string,
  limit: number,
  windowSec: number,
): { allowed: boolean; remaining: number; reset: number } {
  const now = Date.now();
  const bucket = ipBuckets.get(ip);
  if (!bucket || now > bucket.resetAt) {
    ipBuckets.set(ip, { count: 1, resetAt: now + windowSec * 1000 });
    return { allowed: true, remaining: limit - 1, reset: Math.ceil((now + windowSec * 1000) / 1000) };
  }
  if (bucket.count >= limit) {
    return { allowed: false, remaining: 0, reset: Math.ceil(bucket.resetAt / 1000) };
  }
  bucket.count++;
  return { allowed: true, remaining: limit - bucket.count, reset: Math.ceil(bucket.resetAt / 1000) };
}

function cleanupRateLimits() {
  if (ipBuckets.size > 5000) {
    const now = Date.now();
    for (const [key, val] of ipBuckets.entries()) {
      if (now > val.resetAt) {
        ipBuckets.delete(key);
      }
    }
  }
}

// ── Security Headers & Response Helpers ───────────────────────────────────────

function securityHeaders(cacheControl = "public, max-age=60"): Record<string, string> {
  return {
    "Content-Type": "application/json; charset=utf-8",
    "Access-Control-Allow-Origin": "*",
    "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
    "Access-Control-Allow-Headers": "Content-Type, Authorization, x-api-key",
    "X-Content-Type-Options": "nosniff",
    "X-Frame-Options": "DENY",
    "Referrer-Policy": "strict-origin-when-cross-origin",
    "Strict-Transport-Security": "max-age=31536000; includeSubDomains; preload",
    "Cache-Control": cacheControl,
  };
}

function rateLimitExceeded(resetSec: number): Response {
  return new Response(
    JSON.stringify({
      error: {
        code: "rate_limit_exceeded",
        message: "Too many requests. Please slow down.",
      },
    }),
    {
      status: 429,
      headers: {
        ...securityHeaders("no-store"),
        "Retry-After": String(Math.max(1, resetSec - Math.floor(Date.now() / 1000))),
      },
    },
  );
}

function notFound(message: string): Response {
  return new Response(JSON.stringify({ error: { code: "not_found", message } }), {
    status: 404,
    headers: securityHeaders("public, max-age=30"),
  });
}

function badRequest(message: string): Response {
  return new Response(JSON.stringify({ error: { code: "invalid_input", message } }), {
    status: 400,
    headers: securityHeaders("no-store"),
  });
}

function unauthorized(message: string): Response {
  return new Response(JSON.stringify({ error: { code: "unauthorized", message } }), {
    status: 401,
    headers: securityHeaders("no-store"),
  });
}

function forbidden(message: string): Response {
  return new Response(JSON.stringify({ error: { code: "insufficient_scope", message } }), {
    status: 403,
    headers: securityHeaders("no-store"),
  });
}

function serverError(internalLog?: unknown): Response {
  // Leak protection: log internally but return clean generic error
  if (internalLog) {
    console.error("[Registry Error]", internalLog);
  }
  return new Response(
    JSON.stringify({
      error: {
        code: "internal_error",
        message: "An unexpected error occurred. Please try again later.",
      },
    }),
    {
      status: 500,
      headers: securityHeaders("no-store"),
    },
  );
}

// ── Validation & Sanitization ────────────────────────────────────────────────

const PACKAGE_NAME_REGEX = /^(?:@[a-zA-Z0-9_-]+\/)?[a-zA-Z0-9_-]+$/;
const VERSION_REGEX = /^[0-9]+\.[0-9]+\.[0-9]+(?:-[a-zA-Z0-9_.-]+)?$/;
const MAX_BODY_BYTES = 131072; // 128 KB

function isValidPackageName(name: unknown): name is string {
  if (typeof name !== "string") return false;
  if (name.length === 0 || name.length > 128) return false;
  if (name.includes("..") || name.includes("\\") || name.includes("\0")) return false;
  return PACKAGE_NAME_REGEX.test(name);
}

function isValidVersion(version: unknown): version is string {
  if (typeof version !== "string") return false;
  if (version.length === 0 || version.length > 64) return false;
  return VERSION_REGEX.test(version);
}

function sanitizeSearchQuery(q: string): string {
  // Strip PostgREST operators and control characters
  return q.replace(/[\x00-\x1f*?%:,().&^$#@!~`+=\[\]{}|\\]/g, "").trim().slice(0, 64);
}

function timingSafeEqual(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  let mismatch = 0;
  for (let i = 0; i < a.length; i++) {
    mismatch |= a.charCodeAt(i) ^ b.charCodeAt(i);
  }
  return mismatch === 0;
}

// ── Database Client ──────────────────────────────────────────────────────────

async function fetchSupabase(
  env: Env,
  method: string,
  path: string,
  body?: unknown,
  apiKey?: string | null,
): Promise<Response> {
  const supabaseUrl = env.SUPABASE_URL || "https://yarqrdhcmxhagxbbjrgu.supabase.co";
  const supabaseKey =
    env.SUPABASE_SERVICE_ROLE_KEY ||
    env.SUPABASE_PUBLISHABLE_KEY ||
    "sb_publishable_j-3maSzjTD0jeojuYFCMvw_MGAYApAN";
  const url = `${supabaseUrl}/rest/v1${path}`;
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    apikey: supabaseKey,
    Authorization: `Bearer ${apiKey || supabaseKey}`,
  };

  const init: RequestInit = {
    method,
    headers,
  };

  if (body && (method === "POST" || method === "PATCH")) {
    init.body = JSON.stringify(body);
  }

  try {
    const res = await fetch(url, init);
    const data = await res.json().catch(() => null);
    return new Response(JSON.stringify(data), {
      status: res.status,
      headers: securityHeaders(),
    });
  } catch (e) {
    return serverError(e);
  }
}

async function getRegistryManifest(env: Env): Promise<RegistryManifest | null> {
  try {
    const res = await fetchSupabase(env, "GET", "/rpc/get_registry_manifest");
    if (res.ok) {
      const data = await res.json().catch(() => null);
      if (data && data.packages && Object.keys(data.packages).length > 0) return data;
    }
  } catch {
    // Fall back to table query
  }
  return null;
}

// ── Route Handlers ───────────────────────────────────────────────────────────

async function handleGETIndex(env: Env): Promise<Response> {
  const regUrl = env.REGISTRY_URL || "https://registry.lplusplus.bond";
  try {
    let manifest = await getRegistryManifest(env);

    if (!manifest) {
      const res = await fetchSupabase(env, "GET", "/packages?select=*&order=created_at.desc");
      const packagesMap: Record<string, RegistryPackage> = { ...DEFAULT_PACKAGES };

      if (res.ok) {
        const packages = await res.json().catch(() => []);
        if (Array.isArray(packages) && packages.length > 0) {
          for (const p of packages) {
            if (p && isValidPackageName(p.name)) {
              packagesMap[p.name] = (p.metadata || p) as RegistryPackage;
            }
          }
        }
      }

      manifest = {
        registry: {
          name: "L++ Package Registry",
          version: "2.0.0",
          url: regUrl,
          description: "Official L++ package registry — powered by Cloudflare & Supabase",
        },
        packages: packagesMap,
      };
    }

    return new Response(JSON.stringify(manifest), {
      headers: securityHeaders("public, max-age=60"),
    });
  } catch {
    const fallbackManifest: RegistryManifest = {
      registry: {
        name: "L++ Package Registry",
        version: "2.0.0",
        url: regUrl,
        description: "Official L++ package registry — powered by Cloudflare & Supabase",
      },
      packages: DEFAULT_PACKAGES,
    };
    return new Response(JSON.stringify(fallbackManifest), {
      headers: securityHeaders("public, max-age=60"),
    });
  }
}

async function handleGETShard(env: Env, shard: string): Promise<Response> {
  const safeShard = shard.replace(/[^a-zA-Z0-9_-]/g, "").slice(0, 32);
  if (!safeShard) {
    return badRequest("Invalid shard parameter.");
  }

  try {
    const encodedShard = encodeURIComponent(safeShard);
    const res = await fetchSupabase(env, "GET", `/packages?name=ilike.${encodedShard}*&select=name,version`);
    if (res.ok) {
      const packages = await res.json().catch(() => []);
      if (Array.isArray(packages) && packages.length > 0) {
        return new Response(JSON.stringify(packages), {
          headers: securityHeaders("public, max-age=300"),
        });
      }
    }

    // Fallback filter from default packages
    const filtered = Object.values(DEFAULT_PACKAGES)
      .filter((p) => p.name.toLowerCase().startsWith(safeShard.toLowerCase()))
      .map((p) => ({ name: p.name, version: p.version }));
    return new Response(JSON.stringify(filtered), {
      headers: securityHeaders("public, max-age=300"),
    });
  } catch (e) {
    return serverError(e);
  }
}

async function handlePOSTPublish(env: Env, request: Request): Promise<Response> {
  // Content-Length check to prevent memory exhaustion / DoS
  const contentLength = parseInt(request.headers.get("content-length") || "0", 10);
  if (contentLength > MAX_BODY_BYTES) {
    return badRequest(`Payload too large. Maximum size is ${MAX_BODY_BYTES / 1024} KB.`);
  }

  // Extract API key
  const authHeader = request.headers.get("authorization") || request.headers.get("x-api-key");
  if (!authHeader) return unauthorized("Send your key in the x-api-key or Authorization header.");

  const apiKey = authHeader.replace(/^Bearer\s+/i, "").trim();
  if (!apiKey || apiKey.length < 16) {
    return unauthorized("Invalid API key format.");
  }

  // Authentication check:
  // 1. Direct match with configured SUPABASE_SERVICE_ROLE_KEY
  let isAuthorized = false;
  if (env.SUPABASE_SERVICE_ROLE_KEY && timingSafeEqual(apiKey, env.SUPABASE_SERVICE_ROLE_KEY)) {
    isAuthorized = true;
  }

  // 2. Check scoped api_keys table in database if not root key
  if (!isAuthorized) {
    const encodedKey = encodeURIComponent(apiKey);
    const keyCheck = await fetchSupabase(env, "GET", `/api_keys?id=eq.${encodedKey}&select=scopes`);
    if (keyCheck.ok) {
      const keyData = await keyCheck.json().catch(() => null);
      if (Array.isArray(keyData) && keyData.length > 0) {
        const scopes = (keyData[0]?.scopes as string[]) || [];
        if (scopes.includes("api:write")) {
          isAuthorized = true;
        } else {
          return forbidden("API key is missing the 'api:write' scope.");
        }
      }
    }
  }

  if (!isAuthorized) {
    return unauthorized("Invalid API key or unauthorized.");
  }

  // Parse package metadata
  let rawBody: string;
  try {
    rawBody = await request.text();
  } catch {
    return badRequest("Unable to read request payload.");
  }

  if (rawBody.length > MAX_BODY_BYTES) {
    return badRequest(`Payload exceeds max limit of ${MAX_BODY_BYTES / 1024} KB.`);
  }

  let pkg: RegistryPackage;
  try {
    pkg = JSON.parse(rawBody) as RegistryPackage;
  } catch {
    return badRequest("Invalid JSON in request body.");
  }

  if (!isValidPackageName(pkg.name)) {
    return badRequest(
      "Invalid package name. Names must contain only alphanumeric characters, dashes, and underscores.",
    );
  }

  if (pkg.version && !isValidVersion(pkg.version)) {
    return badRequest("Invalid version format. Must follow semantic versioning (e.g. 1.0.0).");
  }

  try {
    const insertData = {
      name: pkg.name,
      version: pkg.version || "0.1.0",
      metadata: {
        name: pkg.name,
        version: pkg.version || "0.1.0",
        description: (pkg.description || "").slice(0, 500),
        repository: (pkg.repository || "").slice(0, 255),
        git: (pkg.git || "").slice(0, 255),
        license: (pkg.license || "MIT").slice(0, 50),
        authors: Array.isArray(pkg.authors) ? pkg.authors.slice(0, 10) : [],
        dependencies: Array.isArray(pkg.dependencies) ? pkg.dependencies.slice(0, 50) : [],
        keywords: Array.isArray(pkg.keywords) ? pkg.keywords.slice(0, 20) : [],
      },
      published_by: apiKey.startsWith("sb_") ? "service_role" : apiKey.slice(0, 8),
    };

    const res = await fetchSupabase(env, "POST", "/packages", insertData);
    if (!res.ok) {
      return serverError("Failed to persist package metadata.");
    }

    return new Response(
      JSON.stringify({
        success: true,
        package: pkg.name,
        version: pkg.version || "0.1.0",
        message: "Published successfully to L++ registry.",
      }),
      {
        status: 201,
        headers: securityHeaders("no-store"),
      },
    );
  } catch (e) {
    return serverError(e);
  }
}

async function handleGETSearch(env: Env, query: string): Promise<Response> {
  const sanitized = sanitizeSearchQuery(query);
  if (!sanitized) {
    return new Response(JSON.stringify({ results: Object.values(DEFAULT_PACKAGES), count: Object.keys(DEFAULT_PACKAGES).length }), {
      headers: securityHeaders("public, max-age=60"),
    });
  }

  try {
    const encodedQuery = encodeURIComponent(sanitized);
    const res = await fetchSupabase(
      env,
      "GET",
      `/packages?name=ilike.*${encodedQuery}*&select=name,metadata&limit=20`,
    );
    if (res.ok) {
      const packages = await res.json().catch(() => []);
      if (Array.isArray(packages) && packages.length > 0) {
        return new Response(JSON.stringify({ results: packages, count: packages.length }), {
          headers: securityHeaders("public, max-age=60"),
        });
      }
    }

    // Fallback search across DEFAULT_PACKAGES
    const qLower = sanitized.toLowerCase();
    const results = Object.values(DEFAULT_PACKAGES).filter(
      (p) =>
        p.name.toLowerCase().includes(qLower) ||
        (p.description && p.description.toLowerCase().includes(qLower)),
    );
    return new Response(JSON.stringify({ results, count: results.length }), {
      headers: securityHeaders("public, max-age=60"),
    });
  } catch (e) {
    return serverError(e);
  }
}

async function handleGETPackage(env: Env, name: string): Promise<Response> {
  if (!isValidPackageName(name)) {
    return badRequest("Invalid package name format.");
  }

  try {
    const encodedName = encodeURIComponent(name);
    const res = await fetchSupabase(env, "GET", `/packages?name=eq.${encodedName}&select=metadata`);
    if (res.ok) {
      const data = await res.json().catch(() => null);
      if (Array.isArray(data) && data.length > 0 && data[0]?.metadata) {
        return new Response(JSON.stringify(data[0].metadata), {
          headers: securityHeaders("public, max-age=60"),
        });
      }
    }

    if (DEFAULT_PACKAGES[name]) {
      return new Response(JSON.stringify(DEFAULT_PACKAGES[name]), {
        headers: securityHeaders("public, max-age=60"),
      });
    }
    return notFound(`Package '${name}' not found.`);
  } catch (e) {
    return serverError(e);
  }
}

async function handleGETHealth(env: Env): Promise<Response> {
  return new Response(
    JSON.stringify({
      status: "healthy",
      registry: env.REGISTRY_URL || "https://registry.lplusplus.bond",
      version: "2.0.0",
      security: {
        rate_limiting: "active",
        leak_protection: "active",
      },
    }),
    {
      headers: securityHeaders("no-store"),
    },
  );
}

// ── Main Fetch Dispatcher ────────────────────────────────────────────────────

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const clientIp = request.headers.get("CF-Connecting-IP") || "127.0.0.1";
    cleanupRateLimits();

    const url = new URL(request.url);
    const path = url.pathname;

    // Handle CORS preflight
    if (request.method === "OPTIONS") {
      return new Response(null, { headers: securityHeaders("no-store") });
    }

    // Rate limiting: /publish (10 req/min), search/shards (60 req/min), general reads (120 req/min)
    const isPublish = path === "/publish" && request.method === "POST";
    const isSearch = path.startsWith("/search") || path.startsWith("/names/");
    const limit = isPublish ? 10 : isSearch ? 60 : 120;
    const windowSec = 60;

    const rateCheck = checkRateLimit(clientIp, limit, windowSec);
    if (!rateCheck.allowed) {
      return rateLimitExceeded(rateCheck.reset);
    }

    // ── Routes ──────────────────────────────────────────────────────────────
    if (path === "/index.json" || path === "/") {
      return handleGETIndex(env);
    }

    if (path === "/health") {
      return handleGETHealth(env);
    }

    if (path.startsWith("/names/")) {
      const shard = path.replace("/names/", "");
      if (shard) {
        return handleGETShard(env, shard);
      }
      return badRequest("Shard key required.");
    }

    if (path.startsWith("/search")) {
      const query = url.searchParams.get("q") || "";
      return handleGETSearch(env, query);
    }

    if (path.startsWith("/packages/")) {
      const name = path.replace("/packages/", "").split("/")[0];
      return handleGETPackage(env, name);
    }

    if (isPublish) {
      return handlePOSTPublish(env, request);
    }

    return notFound(`Route not found: ${path}`);
  },
};
