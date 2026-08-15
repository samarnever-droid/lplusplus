/**
 * L++ Package Registry API — Cloudflare Worker
 * Hosted at registry.lplusplus.bond
 *
 * Endpoints:
 *   GET  /index.json        — Full registry manifest
 *   GET  /names/:shard      — Package name shard (for lpp-pm)
 *   POST /publish           — Publish a package to R2
 *   GET  /search?q=query    — Search packages
 *   GET  /packages/:name    — Get package metadata
 */

interface Env {
  REGISTRY_BUCKET: R2Bucket;
  DOMAIN: string;
  REGISTRY_URL: string;
}

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

// ── Registry data — in production, this is loaded from R2 ────────────────────
const INDEX_KEY = "registry/index.json";

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

async function getRegistryIndex(env: Env): Promise<string | null> {
  // Try R2 first
  const object = await env.REGISTRY_BUCKET.get(INDEX_KEY);
  if (object) {
    return await object.text();
  }
  // Fallback: generate from embedded data
  return null;
}

async function putRegistryIndex(env: Env, content: string): Promise<void> {
  await env.REGISTRY_BUCKET.put(INDEX_KEY, content, {
    httpMetadata: { contentType: "application/json" },
  });
}

async function handleGETIndex(env: Env): Promise<Response> {
  try {
    const content = await getRegistryIndex(env);
    if (content) {
      return new Response(content, {
        headers: { ...corsHeaders(), "Cache-Control": "public, max-age=60" },
      });
    }
    return notFound("Registry index not found");
  } catch (e) {
    return serverError(String(e));
  }
}

async function handleGETShard(env: Env, shard: string): Promise<Response> {
  try {
    const key = `registry/names/${shard}.json`;
    const object = await env.REGISTRY_BUCKET.get(key);
    if (object) {
      return new Response(await object.text(), {
        headers: { ...corsHeaders(), "Cache-Control": "public, max-age=300" },
      });
    }
    return notFound(`Shard ${shard} not found`);
  } catch (e) {
    return serverError(String(e));
  }
}

async function handlePOSTPublish(env: Env, request: Request): Promise<Response> {
  // Parse multipart form or JSON body
  const contentType = request.headers.get("content-type") || "";
  let pkg: RegistryPackage;

  try {
    if (contentType.includes("multipart/form-data")) {
      const formData = await request.formData();
      const manifest = formData.get("manifest");
      if (!manifest) return badRequest("Missing manifest field");
      pkg = JSON.parse(String(manifest)) as RegistryPackage;
    } else {
      pkg = (await request.json()) as RegistryPackage;
    }
  } catch {
    return badRequest("Invalid JSON in request body");
  }

  if (!pkg.name || pkg.name.trim().length === 0) {
    return badRequest("Package name is required");
  }

  // Store package metadata in R2
  const pkgKey = `packages/${pkg.name}/manifest.json`;
  const versionKey = `packages/${pkg.name}/versions/${pkg.version || "latest"}.json`;

  try {
    await env.REGISTRY_BUCKET.put(pkgKey, JSON.stringify(pkg, null, 2), {
      httpMetadata: { contentType: "application/json" },
    });
    await env.REGISTRY_BUCKET.put(versionKey, JSON.stringify(pkg, null, 2), {
      httpMetadata: { contentType: "application/json" },
    });

    // Update index
    let manifest: RegistryManifest;
    const existing = await getRegistryIndex(env);
    if (existing) {
      manifest = JSON.parse(existing) as RegistryManifest;
    } else {
      manifest = {
        registry: {
          name: "L++ Package Registry",
          version: "2.0",
          url: env.REGISTRY_URL,
          description: "Official L++ package registry — hosted on Cloudflare R2 + Pages",
        },
        packages: {},
      };
    }
    manifest.packages[pkg.name] = pkg;
    await putRegistryIndex(env, JSON.stringify(manifest, null, 2));

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
    const content = await getRegistryIndex(env);
    if (!content) return notFound("Registry not available");

    const manifest: RegistryManifest = JSON.parse(content);
    const q = query.toLowerCase();
    const results = Object.entries(manifest.packages)
      .filter(([name, pkg]) =>
        name.toLowerCase().includes(q) ||
        (pkg.description && pkg.description.toLowerCase().includes(q)) ||
        (pkg.keywords && pkg.keywords.some(k => k.toLowerCase().includes(q)))
      )
      .map(([name, pkg]) => ({ name, ...pkg }));

    return new Response(JSON.stringify({ results, count: results.length }), {
      headers: { ...corsHeaders(), "Cache-Control": "public, max-age=60" },
    });
  } catch (e) {
    return serverError(String(e));
  }
}

async function handleGETPackage(env: Env, name: string): Promise<Response> {
  try {
    const content = await getRegistryIndex(env);
    if (content) {
      const manifest: RegistryManifest = JSON.parse(content);
      const pkg = manifest.packages[name];
      if (pkg) {
        return new Response(JSON.stringify(pkg), {
          headers: corsHeaders(),
        });
      }
    }
    // Fallback: try R2 directly
    const object = await env.REGISTRY_BUCKET.get(`packages/${name}/manifest.json`);
    if (object) {
      return new Response(await object.text(), { headers: corsHeaders() });
    }
    return notFound(`Package '${name}' not found`);
  } catch (e) {
    return serverError(String(e));
  }
}

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
