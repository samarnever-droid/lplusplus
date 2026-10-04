/* SamarOS — headless boot test.
 *
 * Boots build/samaros.img inside the v86 x86 emulator under Node, drives
 * some synthetic mouse/keyboard input, then reads the kernel back buffer
 * straight out of emulated RAM and writes it to a PNG.  This is how the OS
 * is regression tested without a display.
 *
 *   node test/boot_test.mjs [--seconds 6] [--out shot.png] [--script name]
 */
import { V86 } from "v86";
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, "..");
const BACKBUFFER = 0x400000;

function arg(name, fallback) {
    const i = process.argv.indexOf(`--${name}`);
    return i >= 0 ? process.argv[i + 1] : fallback;
}

const seconds = Number(arg("seconds", 8));
const outFile = arg("out", path.join(ROOT, "build", "screen.png"));
const script = arg("script", "idle");
const width = Number(arg("width", 1024));
const height = Number(arg("height", 768));

function png(rgba, w, h) {
    const raw = Buffer.alloc((w * 4 + 1) * h);
    for (let y = 0; y < h; y++) {
        raw[y * (w * 4 + 1)] = 0;
        for (let x = 0; x < w; x++) {
            const s = (y * w + x) * 4;
            const d = y * (w * 4 + 1) + 1 + x * 4;
            raw[d] = rgba[s + 2];      // stored BGRA in the framebuffer
            raw[d + 1] = rgba[s + 1];
            raw[d + 2] = rgba[s];
            raw[d + 3] = 255;
        }
    }
    const chunk = (type, data) => {
        const len = Buffer.alloc(4);
        len.writeUInt32BE(data.length);
        const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
        const crc = Buffer.alloc(4);
        crc.writeUInt32BE(crc32(body) >>> 0);
        return Buffer.concat([len, body, crc]);
    };
    const ihdr = Buffer.alloc(13);
    ihdr.writeUInt32BE(w, 0);
    ihdr.writeUInt32BE(h, 4);
    ihdr[8] = 8; ihdr[9] = 6; ihdr[10] = 0; ihdr[11] = 0; ihdr[12] = 0;
    return Buffer.concat([
        Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
        chunk("IHDR", ihdr),
        chunk("IDAT", zlib.deflateSync(raw, { level: 6 })),
        chunk("IEND", Buffer.alloc(0)),
    ]);
}

let crcTable = null;
function crc32(buf) {
    if (!crcTable) {
        crcTable = new Int32Array(256);
        for (let n = 0; n < 256; n++) {
            let c = n;
            for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
            crcTable[n] = c;
        }
    }
    let c = -1;
    for (let i = 0; i < buf.length; i++) c = crcTable[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
    return c ^ -1;
}

const emulator = new V86({
    wasm_path: path.join(ROOT, "node_modules/v86/build/v86.wasm"),
    bios: { url: path.join(ROOT, "web/bios/seabios.bin") },
    vga_bios: { url: path.join(ROOT, "web/bios/vgabios.bin") },
    hda: { url: path.join(ROOT, "build/samaros.img"), async: false },
    memory_size: 128 * 1024 * 1024,
    vga_memory_size: 16 * 1024 * 1024,
    autostart: true,
    disable_speaker: true,
});

let serial = "";
emulator.add_listener("serial0-output-byte", (b) => {
    serial += String.fromCharCode(b);
});

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function moveMouseTo(x, y) {
    // the kernel tracks relative deltas; sweep in small steps
    for (let i = 0; i < 40; i++) emulator.bus.send("mouse-delta", [-40, 40]);
    await sleep(120);
    const stepx = Math.round(x / 12), stepy = Math.round(y / 12);
    for (let i = 0; i < 12; i++) {
        emulator.bus.send("mouse-delta", [stepx, -stepy]);
        await sleep(18);
    }
    await sleep(120);
}

async function click() {
    emulator.bus.send("mouse-click", [true, false, false]);
    await sleep(90);
    emulator.bus.send("mouse-click", [false, false, false]);
    await sleep(220);
}

async function type(text) {
    for (const ch of text) {
        emulator.keyboard_send_text(ch);
        await sleep(45);
    }
}

const SCRIPTS = {
    idle: async () => {},
    unlock: async () => {
        await click();
        await sleep(1200);
    },
};

(async () => {
    console.log(`booting SamarOS (script: ${script}, ${seconds}s)…`);
    await sleep(seconds * 1000);

    const run = SCRIPTS[script];
    if (run) await run({ moveMouseTo, click, type, sleep, emulator });

    const mem = emulator.read_memory(BACKBUFFER, width * height * 4);
    const buf = png(Buffer.from(mem), width, height);
    fs.mkdirSync(path.dirname(outFile), { recursive: true });
    fs.writeFileSync(outFile, buf);
    console.log(`wrote ${outFile} (${(buf.length / 1024).toFixed(0)} KiB)`);
    if (serial.trim()) console.log("serial:", serial.trim());
    emulator.stop();
    process.exit(0);
})();
