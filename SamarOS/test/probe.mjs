/* SamarOS — low level boot probe (developer tool).
 * Dumps the boot information block, load addresses and CPU state from a
 * running v86 instance so boot failures can be diagnosed without a screen.
 */
import { V86 } from "v86";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, "..");
const seconds = Number(process.argv[2] || 4);

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
emulator.add_listener("serial0-output-byte", (b) => (serial += String.fromCharCode(b)));

const u32 = (buf, off) =>
    buf[off] | (buf[off + 1] << 8) | (buf[off + 2] << 16) | (buf[off + 3] << 24);

setTimeout(() => {
    const bi = emulator.read_memory(0x5000, 40);
    console.log("bootinfo magic  :", u32(bi, 0).toString(16));
    console.log("  fb addr       : 0x" + (u32(bi, 4) >>> 0).toString(16));
    console.log("  pitch/w/h/bpp :", u32(bi, 8), u32(bi, 12), u32(bi, 16), u32(bi, 20));
    console.log("  mem low kb    :", u32(bi, 24), " high 64k:", u32(bi, 28));
    console.log("  boot drive    : 0x" + u32(bi, 32).toString(16));

    const kbin = fs.readFileSync(path.join(ROOT, "build/kernel.bin"));
    const loaded = Buffer.from(emulator.read_memory(0x20000, 64));
    console.log("kernel@0x20000  :", loaded.subarray(0, 16).toString("hex"));
    console.log("kernel.bin      :", kbin.subarray(0, 16).toString("hex"));
    console.log("kernel match    :", loaded.subarray(0, 64).equals(kbin.subarray(0, 64)));

    const tail = kbin.length - 64;
    const loadedTail = Buffer.from(emulator.read_memory(0x20000 + tail, 64));
    console.log("tail match      :", loadedTail.equals(kbin.subarray(tail, tail + 64)));

    const cpu = emulator.v86.cpu;
    console.log("eip             : 0x" + (cpu.instruction_pointer[0] >>> 0).toString(16));
    console.log("cr0             : 0x" + (cpu.cr[0] >>> 0).toString(16));
    console.log("protected mode  :", (cpu.cr[0] & 1) === 1);
    const back = emulator.read_memory(0x400000, 32);
    console.log("backbuffer[0..8]:", Array.from(back.slice(0, 8)));
    if (serial.trim()) console.log("serial:", JSON.stringify(serial));
    emulator.stop();
    process.exit(0);
}, seconds * 1000);
