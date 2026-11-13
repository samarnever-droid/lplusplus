// Minimal WASI host for running L++ wasm modules under Node.js, used by
// tests/run_wasm_tests.sh when LPP_WASM_RUNTIME=node.
import { readFileSync } from "node:fs";
import { dirname } from "node:path";
import { WASI } from "node:wasi";

const file = process.argv[2];
if (!file) {
  console.error("usage: node node_run.mjs <module.wasm>");
  process.exit(2);
}

// Preopen the module's own directory as `/`, so relative-path file builtins
// (`read_file`, `write_file`, …) resolve next to the case being run. Programs
// that never touch the filesystem are unaffected.
const wasi = new WASI({
  version: "preview1",
  args: [file],
  env: {},
  returnOnExit: true,
  preopens: { [dirname(file)]: dirname(file) },
});

const bytes = readFileSync(file);
const module = await WebAssembly.compile(bytes);
const instance = await WebAssembly.instantiate(module, {
  wasi_snapshot_preview1: wasi.wasiImport,
});
process.exit(wasi.start(instance));
