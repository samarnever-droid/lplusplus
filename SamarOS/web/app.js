// SamarOS — browser front end.
//
// Boots the SamarOS disk image inside the v86 x86 emulator and paints the
// kernel's 32bpp back buffer (at physical 0x400000) onto the #vga canvas.
// Keyboard and mouse are forwarded to the emulated PS/2 devices through the
// v86 event bus.  No v86 screen/keyboard adapter is used: we drive the render
// loop and input entirely ourselves so the page keeps full control (and so
// browser shortcuts such as reload keep working).

import { V86 } from "v86";

const WIDTH = 1024;
const HEIGHT = 768;
const BACKBUFFER = 0x400000;
const FRAME_BYTES = WIDTH * HEIGHT * 4;

const canvas = document.getElementById("vga");
canvas.width = WIDTH;
canvas.height = HEIGHT;
const ctx = canvas.getContext("2d", { alpha: false });
const image = ctx.createImageData(WIDTH, HEIGHT);
const out32 = new Uint32Array(image.data.buffer);

// ---- emulator -------------------------------------------------------------

let emulator = null;
let rafId = null;
let booted = false;

function startEmulator() {
  if (emulator) {
    try { emulator.stop(); } catch (_) {}
    emulator = null;
  }
  booted = false;

  emulator = new V86({
    wasm_path: "/vendor/v86.wasm",
    bios: { url: "/bios/seabios.bin" },
    vga_bios: { url: "/bios/vgabios.bin" },
    hda: { url: "/samaros.img" },
    memory_size: 128 * 1024 * 1024,
    vga_memory_size: 16 * 1024 * 1024,
    autostart: true,
    disable_speaker: true,
    // We handle keyboard + mouse ourselves.
    disable_keyboard: true,
    disable_mouse: true,
  });

  emulator.add_listener("emulator-started", () => { booted = true; });
  emulator.add_listener("emulator-stopped", () => {});

  showOverlay();
  if (rafId) cancelAnimationFrame(rafId);
  renderLoop();
}

// ---- render loop ----------------------------------------------------------

// The kernel stores pixels as 0x00RRGGBB (little-endian memory = B,G,R,0).
// ImageData wants R,G,B,A, so we swap the red and blue bytes.
function renderLoop() {
  rafId = requestAnimationFrame(renderLoop);

  let fb;
  try {
    fb = emulator.read_memory(BACKBUFFER, FRAME_BYTES);
  } catch (_) {
    return; // wasm not ready yet
  }
  if (!fb || fb.length < FRAME_BYTES) return;

  for (let i = 0, j = 0; i < FRAME_BYTES; i += 4, j++) {
    const b = fb[i];
    const g = fb[i + 1];
    const r = fb[i + 2];
    out32[j] = 0xff000000 | (b << 16) | (g << 8) | r;
  }
  ctx.putImageData(image, 0, 0);
}

// ---- keyboard -------------------------------------------------------------
//
// event.code -> PS/2 scan code set 1 (values >= 0x100 carry the 0xE0 prefix).
// Forwarding the raw make/break codes lets the kernel's own scan-code table
// (with its shift / caps-lock state) produce the right character.

const SCAN = {
  Escape: 0x01, Digit1: 0x02, Digit2: 0x03, Digit3: 0x04, Digit4: 0x05,
  Digit5: 0x06, Digit6: 0x07, Digit7: 0x08, Digit8: 0x09, Digit9: 0x0a,
  Digit0: 0x0b, Minus: 0x0c, Equal: 0x0d, Backspace: 0x0e, Tab: 0x0f,
  KeyQ: 0x10, KeyW: 0x11, KeyE: 0x12, KeyR: 0x13, KeyT: 0x14, KeyY: 0x15,
  KeyU: 0x16, KeyI: 0x17, KeyO: 0x18, KeyP: 0x19, BracketLeft: 0x1a,
  BracketRight: 0x1b, Enter: 0x1c, ControlLeft: 0x1d, KeyA: 0x1e, KeyS: 0x1f,
  KeyD: 0x20, KeyF: 0x21, KeyG: 0x22, KeyH: 0x23, KeyJ: 0x24, KeyK: 0x25,
  KeyL: 0x26, Semicolon: 0x27, Quote: 0x28, Backquote: 0x29, ShiftLeft: 0x2a,
  Backslash: 0x2b, KeyZ: 0x2c, KeyX: 0x2d, KeyC: 0x2e, KeyV: 0x2f, KeyB: 0x30,
  KeyN: 0x31, KeyM: 0x32, Comma: 0x33, Period: 0x34, Slash: 0x35,
  IntlRo: 0x35, ShiftRight: 0x36, NumpadMultiply: 0x37, AltLeft: 0x38,
  Space: 0x39, CapsLock: 0x3a, F1: 0x3b, F2: 0x3c, F3: 0x3d, F4: 0x3e,
  F5: 0x3f, F6: 0x40, F7: 0x41, F8: 0x42, F9: 0x43, F10: 0x44,
  NumLock: 0x45, ScrollLock: 0x46, Numpad7: 0x47, Numpad8: 0x48,
  Numpad9: 0x49, NumpadSubtract: 0x4a, Numpad4: 0x4b, Numpad5: 0x4c,
  Numpad6: 0x4d, NumpadAdd: 0x4e, Numpad1: 0x4f, Numpad2: 0x50,
  Numpad3: 0x51, Numpad0: 0x52, NumpadDecimal: 0x53, IntlBackslash: 0x56,
  F11: 0x57, F12: 0x58, NumpadEnter: 0xe00c, ControlRight: 0xe014,
  NumpadDivide: 0xe035, AltRight: 0xe038, Home: 0xe047, ArrowUp: 0xe048,
  PageUp: 0xe049, ArrowLeft: 0xe04b, ArrowRight: 0xe04d, End: 0xe04f,
  ArrowDown: 0xe050, PageDown: 0xe051, Insert: 0xe052, Delete: 0xe053,
  MetaLeft: 0xe05b, OSLeft: 0xe05b, MetaRight: 0xe05c, OSRight: 0xe05c,
  ContextMenu: 0xe05d,
};

function sendScancode(code, down) {
  const s = SCAN[code];
  if (s === undefined) return;
  const bus = emulator.bus;
  if (s >= 0x100) {
    bus.send("keyboard-code", 0xe0);
    bus.send("keyboard-code", down ? (s & 0xff) : (s & 0xff) | 0x80);
  } else {
    bus.send("keyboard-code", down ? s : (s | 0x80));
  }
}

const heldKeys = new Set();

// Browser shortcuts we deliberately let the browser keep (reload, tab, address
// bar, zoom, full screen) instead of forwarding to the guest.
function isBrowserShortcut(e) {
  if (e.key === "F5" || e.key === "F11") return true;
  if (!(e.ctrlKey || e.metaKey)) return false;
  const k = e.key.toLowerCase();
  return k === "r" || k === "l" || k === "t" || k === "w" || k === "n" ||
         (k >= "1" && k <= "9") || k === "=" || k === "-";
}

window.addEventListener("keydown", (e) => {
  if (isBrowserShortcut(e)) return; // let the browser handle it
  if (!SCAN[e.code]) return;
  e.preventDefault();
  if (e.repeat) {
    // honour auto-repeat: release then press so the guest sees a fresh press
    sendScancode(e.code, false);
    sendScancode(e.code, true);
    return;
  }
  sendScancode(e.code, true);
  heldKeys.add(e.code);
});

window.addEventListener("keyup", (e) => {
  if (!SCAN[e.code]) return;
  sendScancode(e.code, false);
  heldKeys.delete(e.code);
});

// Releasing focus can leave a key "stuck" — clear the tracked state.
window.addEventListener("blur", () => {
  for (const code of heldKeys) sendScancode(code, false);
  heldKeys.clear();
});

// ---- mouse ----------------------------------------------------------------

let last = null;

function scale() {
  const rect = canvas.getBoundingClientRect();
  return {
    sx: rect.width ? WIDTH / rect.width : 1,
    sy: rect.height ? HEIGHT / rect.height : 1,
  };
}

canvas.addEventListener("mouseenter", (e) => {
  last = { x: e.clientX, y: e.clientY };
});

canvas.addEventListener("mouseleave", () => {
  last = null;
});

canvas.addEventListener("mousemove", (e) => {
  const { sx, sy } = scale();
  if (last) {
    const dx = (e.clientX - last.x) * sx;
    const dy = (e.clientY - last.y) * sy;
    // v86 convention: positive delta-y moves the pointer *up*, so negate.
    emulator.bus.send("mouse-delta", [Math.round(dx), Math.round(-dy)]);
  }
  last = { x: e.clientX, y: e.clientY };
});

const mouseButtons = [false, false, false]; // left, middle, right

function emitClick(button, down) {
  mouseButtons[button] = down;
  emulator.bus.send("mouse-click", [mouseButtons[0], mouseButtons[1], mouseButtons[2]]);
}

canvas.addEventListener("mousedown", (e) => {
  e.preventDefault();
  emitClick(e.button, true);
});
window.addEventListener("mouseup", (e) => {
  emitClick(e.button, false);
});
canvas.addEventListener("contextmenu", (e) => e.preventDefault());

canvas.addEventListener("wheel", (e) => {
  e.preventDefault();
  const d = e.deltaY < 0 ? -1 : e.deltaY > 0 ? 1 : 0;
  emulator.bus.send("mouse-wheel", [d, 0]);
}, { passive: false });

// Pointer lock gives smooth relative motion when the user clicks the screen
// and wants to "grab" the pointer (handy for the terminal).
canvas.addEventListener("dblclick", () => {
  if (document.pointerLockElement === canvas) document.exitPointerLock();
  else canvas.requestPointerLock?.();
});
document.addEventListener("pointerlockchange", () => {
  if (document.pointerLockElement === canvas) {
    canvas.addEventListener("mousemove", lockedMove);
  } else {
    canvas.removeEventListener("mousemove", lockedMove);
    last = null;
  }
});
function lockedMove(e) {
  const { sx, sy } = scale();
  emulator.bus.send("mouse-delta", [
    Math.round((e.movementX || 0) * sx),
    Math.round(-(e.movementY || 0) * sy),
  ]);
}

// ---- overlay + buttons ----------------------------------------------------

const overlay = document.getElementById("overlay");

function showOverlay() {
  if (overlay) overlay.classList.remove("hidden");
}
function hideOverlay() {
  if (overlay) overlay.classList.add("hidden");
}
if (overlay) {
  overlay.addEventListener("click", hideOverlay);
}

document.getElementById("restart")?.addEventListener("click", () => {
  startEmulator();
  // give the boot screen a moment to show before we let clicks through
  setTimeout(hideOverlay, 4500);
});

document.getElementById("fullscreen")?.addEventListener("click", () => {
  const el = document.querySelector(".bezel") || document.documentElement;
  if (document.fullscreenElement) document.exitFullscreen();
  else el.requestFullscreen?.();
});

// Hide the loading overlay shortly after boot, or as soon as the first frame
// is drawn (whichever comes later).
let firstFrameAt = 0;
setTimeout(() => {
  // safety net: hide even if the started event was missed
  if (booted) hideOverlay();
}, 5000);

// ---- meta -----------------------------------------------------------------

fetch("/meta.json")
  .then((r) => (r.ok ? r.json() : null))
  .then((m) => {
    if (!m) return;
    const lines = document.getElementById("lpp-lines");
    if (lines) lines.textContent = String(m.lpp_lines);
    const size = document.getElementById("imgsize");
    if (size) size.textContent = "disk " + m.disk_label;
  })
  .catch(() => {});

// ---- go -------------------------------------------------------------------

startEmulator();
