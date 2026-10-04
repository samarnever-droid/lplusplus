// Minimal WASI preview1 host for running L++ wasm modules under Node.js,
// used by the Phase 5D gate. Mirrors the v1 `tests/wasm/node_run.mjs`.
import { readFileSync } from "node:fs";
import { WASI } from "node:wasi";

const file = process.argv[2];
if (!file) {
  console.error("usage: node wasm_run.mjs <module.wasm>");
  process.exit(2);
}

const wasi = new WASI({
  version: "preview1",
  args: [file],
  env: {},
  returnOnExit: true,
});

const bytes = readFileSync(file);
const module = await WebAssembly.compile(bytes);
const instance = await WebAssembly.instantiate(module, {
  wasi_snapshot_preview1: wasi.wasiImport,
});
process.exit(wasi.start(instance));
