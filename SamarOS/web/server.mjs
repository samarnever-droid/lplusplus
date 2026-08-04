// SamarOS web shell — static server for the in-browser emulator.
//
// Serves the contents of this directory (index.html, style.css, app.js, the
// disk image, the SeaBIOS / VGABIOS blobs and the bundled v86 runtime) plus a
// small /meta.json endpoint the page uses to fill in the "L++ lines" and disk
// size chips.  Bound to 0.0.0.0 so Arena's live preview proxy can reach it.
//
//   node web/server.mjs            # PORT defaults to 8080
//   PORT=3000 node web/server.mjs

import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.dirname(fileURLToPath(import.meta.url));
const PORT = Number(process.env.PORT) || 8080;
const HOST = process.env.HOST || "0.0.0.0";

// Make sure the bundled v86 runtime is present next to the page.  We copy it
// out of node_modules (where npm puts it) so the preview works even from a
// checkout that only ships web/ — but if node_modules is missing we simply
// fall through and serve whatever is already in web/vendor.
  const VENDOR = path.join(ROOT, "vendor");
  const NM = path.join(ROOT, "..", "node_modules", "v86", "build");
  for (const f of ["v86.mjs", "v86.wasm"]) {
  const dst = path.join(VENDOR, f);
  const src = path.join(NM, f);
  if (!fs.existsSync(dst) && fs.existsSync(src)) {
    fs.mkdirSync(VENDOR, { recursive: true });
    fs.copyFileSync(src, dst);
  }
}

// Likewise make sure a bootable disk image exists (build/ -> web/).
const imgSrc = path.join(ROOT, "..", "build", "samaros.img");
const imgDst = path.join(ROOT, "samaros.img");
if (!fs.existsSync(imgDst) && fs.existsSync(imgSrc)) {
  fs.copyFileSync(imgSrc, imgDst);
}

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".wasm": "application/wasm",
  ".bin": "application/octet-stream",
  ".img": "application/octet-stream",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".ico": "image/x-icon",
  ".map": "application/json; charset=utf-8",
};

function safeJoin(base, target) {
  const p = path.normalize(path.join(base, target));
  if (!p.startsWith(base)) return null; // path traversal guard
  return p;
}

function lppLineCount() {
  const src = path.join(ROOT, "..", "kernel", "src");
  if (!fs.existsSync(src)) return 0;
  let total = 0;
  for (const f of fs.readdirSync(src)) {
    if (f.endsWith(".lpp")) {
      const text = fs.readFileSync(path.join(src, f), "utf8");
      total += text.split("\n").length;
    }
  }
  return total;
}

function metaJson() {
  let size = 0;
  try {
    size = fs.statSync(imgDst).size;
  } catch {
    try {
      size = fs.statSync(imgSrc).size;
    } catch {}
  }
  const mib = size / (1024 * 1024);
  const diskLabel = mib >= 1 ? `${mib.toFixed(1)} MiB` : `${Math.round(size / 1024)} KiB`;
  return {
    lpp_lines: lppLineCount(),
    disk_bytes: size,
    disk_label: diskLabel,
    resolution: [1024, 768],
    bpp: 32,
    built_at: new Date().toISOString(),
  };
}

function send(res, status, body, headers = {}) {
  res.writeHead(status, {
    "Access-Control-Allow-Origin": "*",
    "Cache-Control": "no-cache",
    ...headers,
  });
  res.end(body);
}

const server = http.createServer((req, res) => {
  let urlPath;
  try {
    urlPath = decodeURIComponent((req.url || "/").split("?")[0]);
  } catch {
    urlPath = "/";
  }

  if (urlPath === "/meta.json") {
    return send(res, 200, JSON.stringify(metaJson()), {
      "Content-Type": "application/json; charset=utf-8",
    });
  }

  if (urlPath === "/") urlPath = "/index.html";

  const filePath = safeJoin(ROOT, urlPath);
  if (!filePath) return send(res, 403, "Forbidden");

  fs.stat(filePath, (err, st) => {
    if (err || !st.isFile()) {
      return send(res, 404, "Not found: " + urlPath);
    }
    const ext = path.extname(filePath).toLowerCase();
    const type = MIME[ext] || "application/octet-stream";
    const stream = fs.createReadStream(filePath);
    stream.on("error", () => send(res, 500, "Read error"));
    res.writeHead(200, {
      "Access-Control-Allow-Origin": "*",
      "Content-Type": type,
      "Content-Length": st.size,
      "Cache-Control": "no-cache",
    });
    stream.pipe(res);
  });
});

server.listen(PORT, HOST, () => {
  console.log(`SamarOS web shell listening on http://${HOST}:${PORT}/`);
  console.log(`  v86 runtime : ${fs.existsSync(path.join(VENDOR, "v86.wasm")) ? "present" : "MISSING (copy from node_modules failed)"}`);
  console.log(`  disk image  : ${fs.existsSync(imgDst) ? imgDst : imgSrc} (${metaJson().disk_label})`);
});
