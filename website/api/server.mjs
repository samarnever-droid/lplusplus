// L++ PostgreSQL 17 API Service (ready to deploy on blitz.cloud with DATABASE_URL)
import http from "node:http";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import pg from "pg";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const PORT = Number(process.env.PORT || 8787);
const DATABASE_URL = process.env.DATABASE_URL;

if (!DATABASE_URL) {
  console.error("DATABASE_URL env var is required.");
  process.exit(1);
}

const pool = new pg.Pool({ connectionString: DATABASE_URL });

async function initSchema() {
  const sql = fs.readFileSync(path.join(__dirname, "schema.sql"), "utf8");
  await pool.query(sql);
}

function sendJson(res, status, data) {
  res.writeHead(status, {
    "Content-Type": "application/json; charset=utf-8",
    "Access-Control-Allow-Origin": "*",
    "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
    "Access-Control-Allow-Headers": "Content-Type, Authorization",
  });
  res.end(JSON.stringify(data));
}

async function readBody(req, maxBytes = 65536) {
  let body = "";
  for await (const chunk of req) {
    body += chunk;
    if (body.length > maxBytes) throw new Error("Payload too large");
  }
  return body ? JSON.parse(body) : {};
}

const server = http.createServer(async (req, res) => {
  if (req.method === "OPTIONS") {
    return sendJson(res, 204, {});
  }
  const url = new URL(req.url || "/", `http://${req.headers.host}`);

  try {
    if (req.method === "GET" && url.pathname === "/health") {
      const r = await pool.query("SELECT NOW() AS now");
      return sendJson(res, 200, { status: "ok", db: "postgresql", time: r.rows[0].now });
    }

    // GET /api/stars/:pkg
    const starMatch = url.pathname.match(/^\/api\/stars\/([a-z0-9-]+)$/);
    if (starMatch && req.method === "GET") {
      const pkg = starMatch[1];
      const r = await pool.query(
        "SELECT COUNT(*)::int AS count FROM package_stars WHERE package_name = $1",
        [pkg]
      );
      return sendJson(res, 200, { package: pkg, stars: r.rows[0].count });
    }

    // POST /api/stars/:pkg { userId }
    if (starMatch && req.method === "POST") {
      const pkg = starMatch[1];
      const { userId } = await readBody(req);
      if (!userId || typeof userId !== "string") {
        return sendJson(res, 400, { error: "userId is required" });
      }
      const existing = await pool.query(
        "SELECT 1 FROM package_stars WHERE package_name = $1 AND clerk_user_id = $2",
        [pkg, userId]
      );
      let starred;
      if (existing.rowCount > 0) {
        await pool.query(
          "DELETE FROM package_stars WHERE package_name = $1 AND clerk_user_id = $2",
          [pkg, userId]
        );
        starred = false;
      } else {
        await pool.query(
          "INSERT INTO package_stars (package_name, clerk_user_id) VALUES ($1, $2)",
          [pkg, userId]
        );
        starred = true;
      }
      const count = await pool.query(
        "SELECT COUNT(*)::int AS count FROM package_stars WHERE package_name = $1",
        [pkg]
      );
      return sendJson(res, 200, { package: pkg, starred, stars: count.rows[0].count });
    }

    // POST /api/snippets { title, code, userId? }
    if (req.method === "POST" && url.pathname === "/api/snippets") {
      const { title, code, userId } = await readBody(req);
      if (!code || typeof code !== "string") {
        return sendJson(res, 400, { error: "code is required" });
      }
      const id = crypto.randomBytes(5).toString("base64url");
      await pool.query(
        "INSERT INTO playground_snippets (id, title, code, clerk_user_id) VALUES ($1, $2, $3, $4)",
        [id, String(title || "Untitled Snippet").slice(0, 80), code.slice(0, 32768), userId || null]
      );
      return sendJson(res, 201, { id });
    }

    // GET /api/snippets/:id
    const snipMatch = url.pathname.match(/^\/api\/snippets\/([A-Za-z0-9_-]+)$/);
    if (snipMatch && req.method === "GET") {
      const r = await pool.query(
        "SELECT id, title, code, created_at FROM playground_snippets WHERE id = $1",
        [snipMatch[1]]
      );
      if (r.rowCount === 0) return sendJson(res, 404, { error: "Snippet not found" });
      return sendJson(res, 200, r.rows[0]);
    }

    return sendJson(res, 404, { error: "Not found" });
  } catch (err) {
    return sendJson(res, 500, { error: String(err.message || err) });
  }
});

initSchema()
  .then(() => {
    server.listen(PORT, () => {
      console.log(`lpp-postgres-api listening on :${PORT}`);
    });
  })
  .catch((err) => {
    console.error("Failed to initialize schema:", err);
    process.exit(1);
  });
