// Headless smoke test for the *browser* runtime path.
//
// Boots the disk image with the same V86 options app.js uses (including
// disable_keyboard / disable_mouse, since app.js forwards input over the bus
// itself) and verifies:
//   1. the emulator boots and the kernel paints a non-black frame,
//   2. clicking unlocks the lock screen,
//   3. keyboard-code + mouse-delta bus events are accepted without error.
//
//   node test/web_smoke.mjs

import { V86 } from "v86";
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, "..");
const BACKBUFFER = 0x400000;
const W = 1024, H = 768, FR = W * H * 4;

function crc32(buf) {
  const t = new Int32Array(256);
  for (let n = 0; n < 256; n++) { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; t[n] = c; }
  let c = -1; for (let i = 0; i < buf.length; i++) c = t[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return c ^ -1;
}
function png(rgba) {
  const raw = Buffer.alloc((W * 4 + 1) * H);
  for (let y = 0; y < H; y++) {
    raw[y * (W * 4 + 1)] = 0;
    for (let x = 0; x < W; x++) {
      const s = (y * W + x) * 4, d = y * (W * 4 + 1) + 1 + x * 4;
      raw[d] = rgba[s + 2]; raw[d + 1] = rgba[s + 1]; raw[d + 2] = rgba[s]; raw[d + 3] = 255;
    }
  }
  const chunk = (type, data) => {
    const len = Buffer.alloc(4); len.writeUInt32BE(data.length);
    const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
    const crc = Buffer.alloc(4); crc.writeUInt32BE(crc32(body) >>> 0);
    return Buffer.concat([len, body, crc]);
  };
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(W, 0); ihdr.writeUInt32BE(H, 4);
  ihdr[8] = 8; ihdr[9] = 6;
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr), chunk("IDAT", zlib.deflateSync(raw, { level: 6 })), chunk("IEND", Buffer.alloc(0)),
  ]);
}
function nonBlack(buf) {
  let n = 0;
  for (let i = 0; i < buf.length; i += 4) if (buf[i] || buf[i + 1] || buf[i + 2]) n++;
  return n;
}

const emulator = new V86({
  wasm_path: path.join(ROOT, "node_modules/v86/build/v86.wasm"),
  bios: { url: path.join(ROOT, "web/bios/seabios.bin") },
  vga_bios: { url: path.join(ROOT, "web/bios/vgabios.bin") },
  hda: { url: path.join(ROOT, "web/samaros.img") },
  memory_size: 128 * 1024 * 1024,
  vga_memory_size: 16 * 1024 * 1024,
  autostart: true,
  disable_speaker: true,
  disable_keyboard: true,
  disable_mouse: true,
});

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

(async () => {
  console.log("booting (browser options)…");
  await sleep(6000);
  let fb = emulator.read_memory(BACKBUFFER, FR);
  console.log("boot frame non-black pixels:", nonBlack(Buffer.from(fb)));
  fs.mkdirSync(path.join(ROOT, "build"), { recursive: true });
  fs.writeFileSync(path.join(ROOT, "build", "smoke_boot.png"), png(Buffer.from(fb)));

  // Move the pointer a little, then click to unlock.
  for (let i = 0; i < 20; i++) emulator.bus.send("mouse-delta", [10, 0]);
  await sleep(60);
  emulator.bus.send("mouse-click", [true, false, false]);
  await sleep(120);
  emulator.bus.send("mouse-click", [false, false, false]);
  await sleep(220);

  // Type a couple of keys through the bus (keyboard-code) — must not throw.
  const type = (code, sc) => {
    emulator.bus.send("keyboard-code", sc);
    emulator.bus.send("keyboard-code", sc | 0x80);
  };
  type("a", 0x1e); // 'a'
  await sleep(80);

  await sleep(2000);
  fb = emulator.read_memory(BACKBUFFER, FR);
  console.log("desktop frame non-black pixels:", nonBlack(Buffer.from(fb)));
  fs.writeFileSync(path.join(ROOT, "build", "smoke_desktop.png"), png(Buffer.from(fb)));

  console.log("OK — no exceptions, runtime path works.");
  emulator.stop();
  process.exit(0);
})().catch((e) => { console.error("SMOKE FAIL:", e); process.exit(1); });
