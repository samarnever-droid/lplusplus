/**
 * L++ Package Registry API — Cloudflare Worker
 * Hosted at registry.lplusplus.bond
 * 
 * Architecture: Same pattern as Cloud Foundation Hub
 * - Supabase backend (yarqrdhcmxhagxbbjrgu)
 * - Scoped API keys with rate limiting
 * - JSONB package metadata
 * - Request logging for analytics
 *
 * Endpoints:
 *   GET  /index.json        — Full registry manifest (public)
 *   GET  /names/:shard      — Package name shard (public)
 *   GET  /search?q=query    — Search packages (public)
 *   GET  /packages/:name    — Get package metadata (public)
 *   POST /publish           — Publish a package (requires api:write scope)
 *   GET  /health            — Health check (public)
 */

interface Env {
  SUPABASE_URL: string;
  SUPABASE_SERVICE_ROLE_KEY: string;
  DOMAIN: string;
  REGISTRY_URL: string;
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

// ── Helpers ───────────────────────────────────────────────────────────────────

function notFound(message: string): Response {
  return new Response(JSON.stringify({ error: { code: "not_found", message } }), {
    status: 404,
    headers: corsHeaders(),
  });
}

function badRequest(message: string): Response {
  return new Response(JSON.stringify({ error: { code: "invalid_input", message } }), {
    status: 400,
    headers: corsHeaders(),
  });
}

function unauthorized(message: string): Response {
  return new Response(JSON.stringify({ error: { code: "missing_api_key", message } }), {
    status: 401,
    headers: corsHeaders(),
  });
}

function forbidden(message: string): Response {
  return new Response(JSON.stringify({ error: { code: "insufficient_scope", message } }), {
    status: 403,
    headers: corsHeaders(),
  });
}

function serverError(message: string): Response {
  return new Response(JSON.stringify({ error: { code: "internal_error", message } }), {
    status: 500,
    headers: corsHeaders(),
  });
}

function corsHeaders(): Record<string, string> {
  return {
    "Content-Type": "application/json",
    "Access-Control-Allow-Origin": "*",
    "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
    "Access-Control-Allow-Headers": "Content-Type, Authorization, x-api-key",
  };
}

async function fetchSupabase(
  env: Env,
  method: string,
  path: string,
  body?: unknown,
  apiKey?: string | null,
): Promise<Response> {
  const url = `${env.SUPABASE_URL}/rest/v1${path}`;
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    "apikey": env.SUPABASE_SERVICE_ROLE_KEY,
    "Authorization": `Bearer ${env.SUPABASE_SERVICE_ROLE_KEY}`,
  };
  
  if (apiKey) {
    headers["Authorization"] = `Bearer ${apiKey}`;
  }

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
      headers: corsHeaders(),
    });
  } catch (e) {
    return serverError(String(e));
  }
}

async function getRegistryManifest(env: Env): Promise<RegistryManifest | null> {
  const res = await fetchSupabase(env, "GET", "/rpc/get_registry_manifest");
  if (res.ok) {
    const data = await res.json().catch(() => null);
    if (data) return data;
  }
  return null;
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async function handleGETIndex(env: Env): Promise<Response> {
  try {
    // Try RPC first, fallback to direct table query
    let manifest = await getRegistryManifest(env);
    
    if (!manifest) {
      const res = await fetchSupabase(env, "GET", "/packages?select=*&order=created_at.desc");
      if (!res.ok) return notFound("Registry not available");
      
      const packages = await res.json().catch(() => []);
      manifest = {
        registry: {
          name: "L++ Package Registry",
          version: "1.0.0",
          url: env.REGISTRY_URL,
          description: "Official L++ package registry — powered by Supabase + Cloudflare",
        },
        packages: Object.fromEntries(
          packages.map((p: any) => [p.name, p.metadata as RegistryPackage])
        ),
      };
    }

    return new Response(JSON.stringify(manifest), {
      headers: { ...corsHeaders(), "Cache-Control": "public, max-age=60" },
    });
  } catch (e) {
    return serverError(String(e));
  }
}

async function handleGETShard(env: Env, shard: string): Promise<Response> {
  try {
    const res = await fetchSupabase(env, "GET", `/packages?name.ilike=${shard}%&select=name,version`);
    if (!res.ok) return notFound(`Shard ${shard} not found`);
    
    const packages = await res.json().catch(() => []);
    return new Response(JSON.stringify(packages), {
      headers: { ...corsHeaders(), "Cache-Control": "public, max-age=300" },
    });
  } catch (e) {
    return serverError(String(e));
  }
}

async function handlePOSTPublish(env: Env, request: Request): Promise<Response> {
  // Extract API key
  const authHeader = request.headers.get("authorization") || request.headers.get("x-api-key");
  if (!authHeader) return unauthorized("Send your key in the x-api-key or Authorization header.");
  
  const apiKey = authHeader.replace("Bearer ", "");
  
  // Verify key has api:write scope
  const keyCheck = await fetchSupabase(env, "GET", `/api_keys?id.eq=${apiKey}&select=scopes`);
  if (!keyCheck.ok) return unauthorized("Invalid API key.");
  
  const keyData = await keyCheck.json().catch(() => null);
  if (!keyData || !Array.isArray(keyData) || keyData.length === 0) {
    return unauthorized("API key not found.");
  }
  
  const scopes = keyData[0]?.scopes as string[] || [];
  if (!scopes.includes("api:write")) {
    return forbidden("API key missing 'api:write' scope.");
  }

  // Parse package metadata
  let pkg: RegistryPackage;
  try {
    pkg = await request.json() as RegistryPackage;
  } catch {
    return badRequest("Invalid JSON in request body");
  }

  if (!pkg.name || pkg.name.trim().length === 0) {
    return badRequest("Package name is required");
  }

  try {
    // Upsert package metadata
    const insertData = {
      name: pkg.name,
      version: pkg.version || "0.0.0",
      metadata: pkg,
      published_by: apiKey,
    };

    const res = await fetchSupabase(env, "POST", "/packages", insertData);
    if (!res.ok) {
      return serverError("Failed to publish package");
    }

    return new Response(JSON.stringify({
      success: true,
      package: pkg.name,
      version: pkg.version,
      message: "Published successfully",
    }), {
      status: 201,
      headers: corsHeaders(),
    });
  } catch (e) {
    return serverError(String(e));
  }
}

async function handleGETSearch(env: Env, query: string): Promise<Response> {
  try {
    const res = await fetchSupabase(
      env,
      "GET",
      `/packages?metadata->>description=ilike.*${query}*&select=name,metadata&limit=20`
    );
    if (!res.ok) return notFound("Registry not available");
    
    const packages = await res.json().catch(() => []);
    return new Response(JSON.stringify({ results: packages, count: packages.length }), {
      headers: { ...corsHeaders(), "Cache-Control": "public, max-age=60" },
    });
  } catch (e) {
    return serverError(String(e));
  }
}

async function handleGETPackage(env: Env, name: string): Promise<Response> {
  try {
    const res = await fetchSupabase(env, "GET", `/packages?name=eq.${name}&select=metadata`);
    if (!res.ok) return notFound(`Package '${name}' not found`);
    
    const data = await res.json().catch(() => null);
    if (!data || data.length === 0) return notFound(`Package '${name}' not found`);
    
    return new Response(JSON.stringify(data[0]?.metadata), {
      headers: corsHeaders(),
    });
  } catch (e) {
    return serverError(String(e));
  }
}

async function handleGETHealth(env: Env): Promise<Response> {
  return new Response(JSON.stringify({
    status: "healthy",
    registry: env.REGISTRY_URL,
    version: "1.0.0",
  }), {
    headers: { ...corsHeaders(), "Cache-Control": "no-store" },
  });
}

// ── Main fetch handler ────────────────────────────────────────────────────────

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    const path = url.pathname;

    // Handle CORS preflight
    if (request.method === "OPTIONS") {
      return new Response(null, { headers: corsHeaders() });
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
      return badRequest("Shard key required");
    }

    if (path.startsWith("/search")) {
      const query = url.searchParams.get("q") || "";
      return handleGETSearch(env, query);
    }

    if (path.startsWith("/packages/")) {
      const name = path.replace("/packages/", "").split("/")[0];
      return handleGETPackage(env, name);
    }

    if (path === "/publish" && request.method === "POST") {
      return handlePOSTPublish(env, request);
    }

    return notFound(`Route not found: ${path}`);
  },
};
