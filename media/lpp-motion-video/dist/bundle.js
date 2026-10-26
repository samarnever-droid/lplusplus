"use strict";
(() => {
  // media/lpp-motion-video/src/engine.ts
  var MORPH = [15, 0.84];
  var FAST = [27, 0.86];
  var SLOW = [12.5, 0.9];
  var CAM = [7.5, 1];
  function clamp(x, a = 0, b = 1) {
    return Math.min(b, Math.max(a, x));
  }
  function lerp(a, b, u) {
    return a + (b - a) * u;
  }
  function seg(t, a, b) {
    if (Math.abs(b - a) < 1e-6) return t >= b ? 1 : 0;
    return clamp((t - a) / (b - a), 0, 1);
  }
  function eo(x) {
    const u = clamp(x);
    return 1 - Math.pow(1 - u, 3);
  }
  function eio(x) {
    const u = clamp(x);
    return u * u * u * (u * (6 * u - 15) + 10);
  }
  function easeInOutCubic(x) {
    const u = clamp(x);
    return u < 0.5 ? 4 * u * u * u : 1 - Math.pow(-2 * u + 2, 3) / 2;
  }
  function spring(tau, w = 14, z = 0.82) {
    if (tau <= 0) return 0;
    if (!isFinite(w)) return 1;
    if (z < 1) {
      const wd = w * Math.sqrt(1 - z * z);
      return 1 - Math.exp(-z * w * tau) * (Math.cos(wd * tau) + z * w / wd * Math.sin(wd * tau));
    }
    if (z > 1 + 1e-6) {
      const q = w * Math.sqrt(z * z - 1);
      const r1 = -z * w + q;
      const r2 = -z * w - q;
      return 1 - (r2 * Math.exp(r1 * tau) - r1 * Math.exp(r2 * tau)) / (r2 - r1);
    }
    return 1 - Math.exp(-w * tau) * (1 + w * tau);
  }
  function track(v0, keys, def = MORPH) {
    let prev = v0;
    const ks = [];
    for (const k of keys) {
      const d = k[1] - prev;
      prev = k[1];
      const sp = k[2] || def;
      if (d !== 0) ks.push([k[0], d, sp[0], sp[1]]);
    }
    return (t) => {
      let v = v0;
      for (const k of ks) {
        if (t > k[0]) v += k[1] * spring(t - k[0], k[2], k[3]);
      }
      return v;
    };
  }
  function hexToRgb(h) {
    const clean = h.replace("#", "");
    return [
      parseInt(clean.slice(0, 2), 16),
      parseInt(clean.slice(2, 4), 16),
      parseInt(clean.slice(4, 6), 16)
    ];
  }
  function ctrack(h0, keys, def = [20, 1]) {
    const c0 = hexToRgb(h0);
    const tr = [0, 1, 2].map(
      (i) => track(
        c0[i],
        keys.map((k) => [k[0], hexToRgb(k[1])[i], k[2]]),
        def
      )
    );
    return (t) => `rgb(${tr.map((f) => Math.round(clamp(f(t), 0, 255))).join(",")})`;
  }
  function kf(keys) {
    return (t) => {
      if (t <= keys[0][0]) return keys[0][1];
      for (let i = 1; i < keys.length; i++) {
        const [a, va] = keys[i - 1];
        const [b, vb, fn = easeInOutCubic] = keys[i];
        if (t <= b) return lerp(va, vb, fn((t - a) / (b - a)));
      }
      return keys[keys.length - 1][1];
    };
  }
  function vis(t, tin, tout, opts = {}) {
    const din = opts.din ?? 0.07;
    const lin = opts.lin ?? 0.25;
    const lout = opts.lout ?? 0.13;
    const bl = opts.blur ?? 10;
    const a = tin == null ? 1 : eo((t - tin - din) / lin);
    const b = tout == null ? 0 : eo((t - tout) / lout);
    return {
      o: a * (1 - b),
      blur: (1 - a) * bl + b * bl * 0.8,
      s: (0.94 + 0.06 * a) * (1 - 0.03 * b),
      a,
      b
    };
  }
  function applyVis(el, v, extraTransform = "") {
    if (!el) return false;
    if (v.o < 2e-3) {
      if (el.style.display !== "none") el.style.display = "none";
      return false;
    }
    if (el.style.display === "none") el.style.display = "";
    el.style.opacity = v.o.toFixed(4);
    el.style.filter = v.blur > 0.08 ? `blur(${v.blur.toFixed(2)}px)` : "none";
    el.style.transform = `scale(${v.s.toFixed(4)})${extraTransform ? " " + extraTransform : ""}`;
    return true;
  }
  function setText(el, s) {
    if (!el) return;
    const anyEl = el;
    if (anyEl._t !== s) {
      el.textContent = s;
      anyEl._t = s;
    }
  }
  function cursorPath(keys) {
    return (t) => {
      if (t <= keys[0][0]) return { x: keys[0][1], y: keys[0][2] };
      const last = keys[keys.length - 1];
      if (t >= last[0]) return { x: last[1], y: last[2] };
      let i = 0;
      while (i + 1 < keys.length && t >= keys[i + 1][0]) i++;
      const [t0, x0, y0] = keys[i];
      const [t1, x1, y1] = keys[i + 1];
      const u = eio((t - t0) / Math.max(1e-4, t1 - t0));
      const dx = x1 - x0;
      const dy = y1 - y0;
      const arc = Math.sin(Math.PI * u) * 0.06;
      return {
        x: x0 + dx * u - dy * arc,
        y: y0 + dy * u + dx * arc
      };
    };
  }
  function cursorPresses(clicks = [], drags = []) {
    const k = [];
    for (const c of clicks) {
      k.push([c - 0.07, 1, [45, 1]], [c + 0.04, 0, [22, 0.72]]);
    }
    for (const [a, b] of drags) {
      k.push([a - 0.04, 1, [45, 1]], [b, 0, [22, 0.72]]);
    }
    k.sort((a, b) => a[0] - b[0]);
    return track(0, k);
  }
  var IC = {
    arrow: ["M5 12h14", "M13 5l7 7-7 7"],
    check: ["M20 6 9 17l-5-5"],
    x: ["M18 6 6 18", "M6 6l12 12"],
    plus: ["M5 12h14", "M12 5v14"],
    terminal: ["m4 17 6-6-6-6", "M12 19h8"],
    file: ["M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z", "M14 2v4a2 2 0 0 0 2 2h4", "M8 13h8", "M8 17h5"],
    folder: ["M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"],
    clock: ["M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20Z", "M12 6v6l4 2"],
    chip: ["M6 4h12a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2Z", "M9 9h6v6H9z", "M9 1v3", "M15 1v3", "M9 20v3", "M15 20v3", "M20 9h3", "M20 14h3", "M1 9h3", "M1 14h3"],
    sparkle: ["M9.94 15.5A2 2 0 0 0 8.5 14.06l-6.14-1.58a.5.5 0 0 1 0-.96L8.5 9.94A2 2 0 0 0 9.94 8.5l1.58-6.14a.5.5 0 0 1 .96 0L14.06 8.5A2 2 0 0 0 15.5 9.94l6.14 1.58a.5.5 0 0 1 0 .96L15.5 14.06a2 2 0 0 0-1.44 1.44l-1.58 6.14a.5.5 0 0 1-.96 0z"],
    shield: ["M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"],
    package: ["m7.5 4.27 9 5.15", "M21 8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16Z", "m3.3 7 8.7 5 8.7-5", "M12 22V12"],
    layers: ["m12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83Z", "m22 17.65-9.17 4.16a2 2 0 0 1-1.66 0L2 17.65", "m22 12.65-9.17 4.16a2 2 0 0 1-1.66 0L2 12.65"]
  };
  function iconSvg(name, size, color, sw = 2.2) {
    const paths = IC[name] || IC.sparkle;
    const strokeW = (sw * 24 / size).toFixed(3);
    return `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="${color}" stroke-width="${strokeW}" stroke-linecap="round" stroke-linejoin="round">${paths.map((d) => `<path d="${d}"/>`).join("")}</svg>`;
  }

  // media/lpp-motion-video/src/data.ts
  var VIDEO_DURATION = 84.5;
  var VO_CUES = [
    {
      id: "vo_01",
      start: 0.8,
      end: 7.12,
      text: "Every second spent waiting on a compiler is a second not spent building. Meet L++.",
      scene: 1
    },
    {
      id: "vo_02",
      start: 8.12,
      end: 16.73,
      text: "A compiled systems language with clean, indented syntax that reads like a script, and runs like native code.",
      scene: 2
    },
    {
      id: "vo_03",
      start: 17.23,
      end: 22.1,
      text: "Write it, build it, and get a real native executable in milliseconds.",
      scene: 2
    },
    {
      id: "vo_04",
      start: 23.1,
      end: 28.56,
      text: "Memory is handled for you. No manual frees. No dangling pointers to chase.",
      scene: 3
    },
    {
      id: "vo_05",
      start: 29.06,
      end: 39.3,
      text: "Reference counting is planned at compile time. Values that never escape stay on the stack, and a cycle collector reclaims anything that loops back on itself.",
      scene: 3
    },
    {
      id: "vo_06",
      start: 39.9,
      end: 47.84,
      text: "Under the hood is one verified pipeline. Parse, type check, build MIR, and prove ownership.",
      scene: 4
    },
    {
      id: "vo_07",
      start: 48.34,
      end: 55.93,
      text: "Then Cranelift for instant native builds, with LLVM and WebAssembly backends alongside.",
      scene: 4
    },
    {
      id: "vo_08",
      start: 56.43,
      end: 64.19,
      text: "Its own linker writes ELF, PE, and Mach-O directly. And Keel manages your packages and workspaces.",
      scene: 5
    },
    {
      id: "vo_09",
      start: 65.19,
      end: 70.74,
      text: "And it's honest about safety. Every ownership proof is checked independently.",
      scene: 6
    },
    {
      id: "vo_10",
      start: 71.24,
      end: 76.62,
      text: "Anything the compiler can't prove, it rejects, before a binary ever exists.",
      scene: 6
    },
    {
      id: "vo_11",
      start: 77.62,
      end: 81.29,
      text: "L++. Write it. Prove it. Ship it.",
      scene: 7
    }
  ];
  var SCENES = [
    { index: 1, tag: "SCENE 01 / 07", category: "SPARK // BRAND SLAM", start: 0, end: 7.75 },
    { index: 2, tag: "SCENE 02 / 07", category: "SYNTAX // MILLISECOND BUILD", start: 7.75, end: 22.65 },
    { index: 3, tag: "SCENE 03 / 07", category: "MEMORY // COMPILE-TIME ARC", start: 22.65, end: 39.6 },
    { index: 4, tag: "SCENE 04 / 07", category: "PIPELINE // MULTI-BACKEND", start: 39.6, end: 56.1 },
    { index: 5, tag: "SCENE 05 / 07", category: "DIRECT LINKER // KEEL PM", start: 56.1, end: 64.75 },
    { index: 6, tag: "SCENE 06 / 07", category: "SAFETY S2 // PROOF VERIFIER", start: 64.75, end: 77.15 },
    { index: 7, tag: "SCENE 07 / 07", category: "KINETIC TRIPLET // END CARD", start: 77.15, end: 84.5 }
  ];
  var LPP_CODE_LINES = [
    { indent: 0, tokens: [["# main.lpp \u2014 Indented, statically typed L++", "cm"]] },
    { indent: 0, tokens: [["struct ", "kw"], ["Point", "ty"], [":", "op"]] },
    { indent: 1, tokens: [["x", "id"], [": ", "op"], ["Float", "ty"]] },
    { indent: 1, tokens: [["y", "id"], [": ", "op"], ["Float", "ty"], [" = ", "op"], ["0.0", "nu"]] },
    { indent: 0, tokens: [["impl ", "kw"], ["Point", "ty"], [":", "op"]] },
    { indent: 1, tokens: [["def ", "kw"], ["magnitude", "fn"], ["(self) -> ", "op"], ["Float", "ty"], [":", "op"]] },
    { indent: 2, tokens: [["return ", "kw"], ["sqrt", "fn"], ["(self.x * self.x + self.y * self.y)", "id"]] },
    { indent: 0, tokens: [["def ", "kw"], ["main", "fn"], ["():", "op"]] },
    { indent: 1, tokens: [["p ", "id"], [":= ", "kw"], ["Point", "ty"], ["(", "op"], ["3.0", "nu"], [", ", "op"], ["4.0", "nu"], [")", "op"]] },
    { indent: 1, tokens: [["dist ", "id"], [":= ", "kw"], ["p.", "id"], ["magnitude", "fn"], ["()", "op"]] },
    { indent: 1, tokens: [["print_str", "fn"], ["(", "op"], ['f"Distance: {dist}"', "st"], [")", "op"]] }
  ];

  // media/lpp-motion-video/src/main.ts
  var W = 1920;
  var H = 1080;
  var CX = W / 2;
  var CY = H / 2;
  window.VIDEO = { w: W, h: H, duration: VIDEO_DURATION };
  window.DURATION = VIDEO_DURATION;
  var $ = (id) => document.getElementById(id);
  var SH = {
    seed: { w: 76, h: 76, r: 38, bg: "#FF4F1A", cam: 1, border: "#FF4F1A" },
    editor: { w: 1180, h: 620, r: 26, bg: "#111318", cam: 1.18, border: "#262934" },
    build: { w: 1260, h: 650, r: 28, bg: "#111318", cam: 1.14, border: "#FF4F1A" },
    memClean: { w: 1080, h: 550, r: 30, bg: "#FFFFFF", cam: 1.24, border: "#E2E0D8" },
    arcWork: { w: 1320, h: 670, r: 28, bg: "#111318", cam: 1.12, border: "#2A2E3D" },
    pipe: { w: 1360, h: 660, r: 30, bg: "#0E1015", cam: 1.1, border: "#252936" },
    linkKeel: { w: 1320, h: 630, r: 28, bg: "#FFFFFF", cam: 1.14, border: "#DFDDD5" },
    safety: { w: 1320, h: 660, r: 28, bg: "#111318", cam: 1.13, border: "#2A2E3D" },
    expand: { w: 1960, h: 1120, r: 0, bg: "#0B0B10", cam: 1, border: "#0B0B10" }
  };
  var SEQ = [
    [7.48, "editor"],
    [17.2, "build"],
    [22.65, "memClean"],
    [28.95, "arcWork"],
    [39.6, "pipe"],
    [56.1, "linkKeel"],
    [64.75, "safety"],
    [77.1, "expand"]
  ];
  var shW = track(SH.seed.w, SEQ.map(([t, k]) => [t, SH[k].w]), MORPH);
  var shH = track(SH.seed.h, SEQ.map(([t, k]) => [t, SH[k].h]), MORPH);
  var shR = track(SH.seed.r, SEQ.map(([t, k]) => [t, SH[k].r]), MORPH);
  var shCam = track(SH.seed.cam, SEQ.map(([t, k]) => [t, SH[k].cam]), CAM);
  var shBG = ctrack(SH.seed.bg, SEQ.map(([t, k]) => [t, SH[k].bg]), [18, 0.95]);
  var shBorder = ctrack(SH.seed.border, SEQ.map(([t, k]) => [t, SH[k].border]), [18, 0.95]);
  var stageBG = ctrack(
    "#0B0B10",
    [
      [1.85, "#FF4F1A", [24, 0.92]],
      [2.55, "#EDEBE4", [18, 0.94]],
      [7.55, "#E9E7E2", [14, 0.95]],
      [28.95, "#E4E2DC", [14, 0.95]],
      [39.6, "#E9E7E2", [14, 0.95]],
      [77.1, "#0B0B10", [24, 0.95]]
    ]
  );
  var camFx = kf([
    [0, 0],
    [8.2, -90],
    [14.5, -40],
    [17.3, 60],
    [22.6, 0],
    [29.2, -50],
    [34, 60],
    [39.6, 0],
    [41, -140],
    [47.2, 120],
    [49, 0],
    [56.1, -90],
    [60.5, 90],
    [64.8, -80],
    [71.2, 85],
    [76.8, 0]
  ]);
  var camFy = kf([
    [0, 0],
    [8.5, -20],
    [15, 25],
    [18, 0],
    [29.5, -20],
    [34.5, 20],
    [40, -35],
    [48.5, 35],
    [56.1, 0],
    [77, 0]
  ]);
  var shapePress = track(0, [
    [18.1 - 0.06, 1, [45, 1]],
    [18.1 + 0.04, 0, [22, 0.72]],
    [24.4 - 0.06, 1, [45, 1]],
    [24.4 + 0.04, 0, [22, 0.72]],
    [61.1 - 0.06, 1, [45, 1]],
    [61.1 + 0.04, 0, [22, 0.72]],
    [73.2 - 0.06, 1.3, [45, 1]],
    [73.2 + 0.05, 0, [20, 0.65]]
  ]);
  var cPath = cursorPath([
    [0, 520, 340],
    [8, 380, 220],
    [12, 240, 110],
    [16.8, 290, 180],
    [17.95, 310, -225],
    // Move to "lpp main.lpp --run" button
    [19.5, 390, 60],
    [22.8, -320, -95],
    // Scene 3A: start of hazard strike
    [24.4, 280, -95],
    [25.8, 280, 15],
    [27.2, 280, 125],
    [29.4, -380, 25],
    // Scene 3B: Placement tabs
    [31.8, -380, 95],
    [34.2, -380, 165],
    [36, 290, 120],
    // Cycle collector node
    [40.2, -460, -145],
    // Scene 4: Pipeline stations
    [42.2, -450, -110],
    [43.4, -155, -110],
    [44.7, 145, -110],
    [46.1, 445, -110],
    [49, 340, 55],
    // Cranelift lane
    [51.5, 340, 140],
    // LLVM lane
    [52.8, 340, 225],
    // WASM lane
    [57.4, -310, -40],
    // Scene 5: ELF / PE / Mach-O
    [58.4, -310, 55],
    [59.3, -310, 150],
    [61, 310, -175],
    // Keel workspace build button
    [63, 280, 95],
    [65.5, -290, -40],
    // Scene 6: Ownership proof checks
    [68, -290, 60],
    [71.5, 290, 10],
    // Rejection shield
    [76.8, 560, 360]
  ]);
  var cPress = cursorPresses(
    [18.1, 24.4, 25.8, 27.2, 31.8, 34.2, 42.2, 43.4, 44.7, 46.1, 57.6, 58.4, 59.2, 61.1, 66.2, 68.1, 73.2],
    [[24.2, 27.3]]
  );
  var buildLead = track(0, [[18.12, 100, FAST]]);
  var buildTrail = track(0, [[18.12, 100, SLOW]]);
  var tierTop = track(0, [
    [29.8, 0, FAST],
    [31.8, 72, SLOW],
    [34.2, 144, SLOW]
  ]);
  var tierBot = track(58, [
    [29.8, 58, SLOW],
    [31.8, 130, FAST],
    [34.2, 202, FAST]
  ]);
  var pipeHeroX = track(-450, [
    [41.8, -450, FAST],
    [43.2, -150, MORPH],
    [44.5, 150, MORPH],
    [45.9, 450, MORPH]
  ]);
  function initDom() {
    document.querySelectorAll("[data-icon]").forEach((el) => {
      const name = el.dataset.icon || "sparkle";
      const size = Number(el.dataset.size || 20);
      const color = el.dataset.color || "currentColor";
      const sw = Number(el.dataset.sw || 2.2);
      el.innerHTML = iconSvg(name, size, color, sw);
    });
    const codeBox = $("s2-code-lines");
    codeBox.innerHTML = LPP_CODE_LINES.map((line, idx) => {
      const pad = "&nbsp;".repeat(line.indent * 4);
      return `<div class="code-line" id="s2-line-${idx}"><span class="ln">${String(idx + 1).padStart(2, "0")}</span><span class="lc">${pad}<span id="s2-tok-${idx}"></span></span></div>`;
    }).join("");
  }
  function updateTypedCode(charBudget, t) {
    let rem = Math.max(0, Math.floor(charBudget));
    let caretPlaced = false;
    const caretOn = Math.floor(t * 3) % 2 === 0;
    for (let i = 0; i < LPP_CODE_LINES.length; i++) {
      const rowEl = $(`s2-line-${i}`);
      const tokContainer = $(`s2-tok-${i}`);
      const tokens = LPP_CODE_LINES[i].tokens;
      const lineLen = tokens.reduce((acc, tk) => acc + tk[0].length, 0);
      if (rem <= 0 && i > 0 && !caretPlaced) {
        rowEl.style.opacity = "0.18";
        setText(tokContainer, "");
        continue;
      }
      rowEl.style.opacity = "1";
      if (rem >= lineLen) {
        rem -= lineLen;
        const html = tokens.map(([txt, cls]) => `<span class="tk-${cls}">${escapeHtml(txt)}</span>`).join("");
        if (tokContainer.dataset.full !== "1") {
          tokContainer.innerHTML = html;
          tokContainer.dataset.full = "1";
        }
      } else {
        tokContainer.dataset.full = "0";
        let left = rem;
        let html = "";
        for (const [txt, cls] of tokens) {
          if (left <= 0) break;
          const part = txt.slice(0, left);
          html += `<span class="tk-${cls}">${escapeHtml(part)}</span>`;
          left -= part.length;
        }
        if (caretOn) {
          html += `<span class="code-caret"></span>`;
        }
        tokContainer.innerHTML = html;
        rem = 0;
        caretPlaced = true;
      }
    }
  }
  function escapeHtml(s) {
    return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }
  function seek(rawT) {
    const t = clamp(rawT, 0, VIDEO_DURATION);
    const stage = $("stage");
    stage.style.background = stageBG(t);
    const gridEl = $("bg-grid");
    const isLightStage = t >= 2.35 && t < 77.1;
    gridEl.style.opacity = isLightStage ? "0.28" : "0.16";
    gridEl.style.backgroundImage = isLightStage ? "radial-gradient(#111318 1.25px, transparent 1.25px)" : "radial-gradient(#FFFFFF 1.25px, transparent 1.25px)";
    const s1Layer = $("scene1-overlay");
    if (t < 7.85) {
      s1Layer.style.display = "";
      const sparkWrap = $("s1-spark-wrap");
      const sparkP = spring(t - 0.12, 18, 0.72);
      const lineExpand = eo(seg(t, 0.45, 1.45));
      const floodExpand = eio(seg(t, 1.8, 2.45));
      const lineW = lerp(14, 1560, lineExpand);
      const lineH = lerp(4, 1120, floodExpand);
      sparkWrap.style.opacity = t < 2.55 ? "1" : "0";
      const sparkBar = $("s1-spark-bar");
      sparkBar.style.width = `${lineW.toFixed(1)}px`;
      sparkBar.style.height = `${lineH.toFixed(1)}px`;
      sparkBar.style.borderRadius = `${lerp(99, 0, floodExpand).toFixed(1)}px`;
      sparkBar.style.transform = `translate(-50%, -50%) scale(${sparkP.toFixed(3)})`;
      const sparkLabel = $("s1-spark-label");
      const slVis = vis(t, 0.35, 1.82, { din: 0.02, lin: 0.2, lout: 0.12 });
      applyVis(sparkLabel, slVis);
      const waitBlock = $("s1-wait-block");
      const wbVis = vis(t, 2.25, 4.35, { din: 0.04, lin: 0.24, lout: 0.15 });
      if (applyVis(waitBlock, wbVis)) {
        const waitSec = (1.4 + (t - 2.25) * 3.8).toFixed(1);
        setText($("s1-wait-timer"), `compiling... ${waitSec}s`);
        const strikeW = eo(seg(t, 3.25, 3.85)) * 100;
        $("s1-wait-strike").style.width = `${strikeW.toFixed(1)}%`;
      }
      const brandBlock = $("s1-brand-block");
      const bbVis = vis(t, 4.32, 7.48, { din: 0.04, lin: 0.24, lout: 0.16 });
      if (applyVis(brandBlock, bbVis)) {
        const slamScale = 0.82 + 0.18 * spring(t - 4.35, 22, 0.68);
        const wght = Math.round(lerp(300, 900, eo(seg(t, 4.35, 5.1))));
        const lppTitle = $("s1-lpp-title");
        lppTitle.style.fontVariationSettings = `'wght' ${wght}`;
        lppTitle.style.transform = `scale(${slamScale.toFixed(4)})`;
      }
    } else {
      s1Layer.style.display = "none";
    }
    const world = $("world");
    const shape = $("shape");
    const cur = $("cursor");
    if (t >= 7.35 && t < 77.25) {
      world.style.display = "";
      cur.style.display = "";
      const w = shW(t);
      const h = shH(t);
      const r = Math.min(shR(t), w / 2, h / 2);
      const s = shCam(t);
      const fx = camFx(t);
      const fy = camFy(t);
      const sc = (0.4 + 0.6 * spring(t - 7.35, 16, 0.78)) * (1 - 0.028 * shapePress(t));
      const op = clamp((t - 7.35) / 0.15) * clamp((77.25 - t) / 0.15);
      world.style.transform = `translate(${(CX - s * fx).toFixed(2)}px, ${(CY - s * fy).toFixed(2)}px) scale(${s.toFixed(4)})`;
      shape.style.left = `${(-w / 2).toFixed(2)}px`;
      shape.style.top = `${(-h / 2).toFixed(2)}px`;
      shape.style.width = `${w.toFixed(2)}px`;
      shape.style.height = `${h.toFixed(2)}px`;
      shape.style.borderRadius = `${r.toFixed(2)}px`;
      shape.style.background = shBG(t);
      shape.style.borderColor = shBorder(t);
      shape.style.transform = `scale(${sc.toFixed(4)})`;
      shape.style.opacity = op.toFixed(4);
      const l2 = $("L-scene2");
      const v2 = vis(t, 7.55, 22.65);
      if (applyVis(l2, v2)) {
        l2.style.left = `${(w / 2).toFixed(1)}px`;
        l2.style.top = `${(h / 2).toFixed(1)}px`;
        const chars = Math.floor(lerp(0, 275, clamp((t - 8.2) / 8)));
        updateTypedCode(chars, t);
        const btn = $("s2-build-btn");
        const clicked = t >= 18.1;
        btn.style.background = clicked ? "#10B981" : "#FF4F1A";
        setText($("s2-build-label"), clicked ? "Built \xB7 main.exe \u2713" : "lpp main.lpp --run");
        const lead = buildLead(t);
        const trail = buildTrail(t);
        const barFill = $("s2-bar-fill");
        barFill.style.width = `${clamp(lead, 0, 100).toFixed(1)}%`;
        barFill.style.opacity = t >= 18.05 ? "1" : "0.2";
        const msVal = t < 18.1 ? "0.0" : (3.4 * eo(seg(t, 18.1, 18.85))).toFixed(1);
        setText($("s2-ms-num"), `${msVal} ms`);
        const exeCard = $("s2-exe-card");
        const exeVis = vis(t, 18.5, 22.6, { din: 0.04, lin: 0.22, lout: 0.12 });
        applyVis(exeCard, exeVis);
      }
      const l3a = $("L-scene3a");
      const v3a = vis(t, 22.65, 28.95);
      if (applyVis(l3a, v3a)) {
        l3a.style.left = `${(w / 2).toFixed(1)}px`;
        l3a.style.top = `${(h / 2).toFixed(1)}px`;
        const sTimes = [24.25, 25.65, 27.05];
        for (let i = 0; i < 3; i++) {
          const p = eo(seg(t, sTimes[i], sTimes[i] + 0.45));
          $(`s3a-strike-${i}`).style.width = `${(p * 100).toFixed(1)}%`;
          const badge = $(`s3a-badge-${i}`);
          if (p > 0.6) {
            badge.textContent = "ELIMINATED \xB7 COMPILE TIME";
            badge.className = "hz-badge done";
          } else {
            badge.textContent = "MANUAL HAZARD";
            badge.className = "hz-badge warn";
          }
        }
      }
      const l3b = $("L-scene3b");
      const v3b = vis(t, 28.95, 39.6);
      if (applyVis(l3b, v3b)) {
        l3b.style.left = `${(w / 2).toFixed(1)}px`;
        l3b.style.top = `${(h / 2).toFixed(1)}px`;
        const topY = tierTop(t);
        const botY = tierBot(t);
        const pill = $("s3b-tier-pill");
        pill.style.top = `${topY.toFixed(1)}px`;
        pill.style.height = `${Math.max(44, botY - topY).toFixed(1)}px`;
        const cycPhase = t < 34.8 ? 0 : t < 36.4 ? 1 : t < 37.6 ? 2 : 3;
        const phaseNames = [
          "1 \xB7 ACTIVE CYCLE (Shared SCC)",
          "2 \xB7 MARK GRAY (Trial Decrement)",
          "3 \xB7 SCAN WHITE (0 External Roots)",
          "4 \xB7 RECLAIMED (0 Leaks \xB7 E4304 PASS)"
        ];
        const phaseColors = ["#FF4F1A", "#F59E0B", "#38BDF8", "#10B981"];
        const statusEl = $("s3b-cycle-status");
        setText(statusEl, phaseNames[cycPhase]);
        statusEl.style.color = phaseColors[cycPhase];
        const rcVals = cycPhase === 0 ? ["rc: 2", "rc: 2", "rc: 2"] : cycPhase === 1 ? ["rc: 1 (gray)", "rc: 1 (gray)", "rc: 1 (gray)"] : cycPhase === 2 ? ["rc: 0 (white)", "rc: 0 (white)", "rc: 0 (white)"] : ["FREED \u2713", "FREED \u2713", "FREED \u2713"];
        for (let i = 0; i < 3; i++) {
          const nd = $(`s3b-node-${i}`);
          setText($(`s3b-rc-${i}`), rcVals[i]);
          nd.style.borderColor = phaseColors[cycPhase];
          const shrink = cycPhase === 3 ? 0.88 : 1;
          nd.style.transform = `scale(${shrink})`;
          nd.style.opacity = cycPhase === 3 ? "0.65" : "1";
        }
      }
      const l4 = $("L-scene4");
      const v4 = vis(t, 39.6, 56.1);
      if (applyVis(l4, v4)) {
        l4.style.left = `${(w / 2).toFixed(1)}px`;
        l4.style.top = `${(h / 2).toFixed(1)}px`;
        const hx = pipeHeroX(t);
        const orb = $("s4-hero-orb");
        orb.style.transform = `translate(${hx.toFixed(1)}px, -125px)`;
        orb.style.opacity = t < 48.2 ? "1" : "0.35";
        const stTimes = [41.8, 43.2, 44.5, 45.9];
        for (let i = 0; i < 4; i++) {
          const active = t >= stTimes[i];
          const stEl = $(`s4-st-${i}`);
          const hit = spring(t - stTimes[i], 24, 0.65);
          stEl.style.borderColor = active ? "#FF4F1A" : "#262A36";
          stEl.style.background = active ? "#181C26" : "#12151E";
          stEl.style.transform = `scale(${(1 + 0.04 * (1 - Math.abs(hit - 0.5) * 2) * (active ? 1 : 0)).toFixed(3)})`;
        }
        const bTimes = [48.6, 51.3, 52.6];
        for (let i = 0; i < 3; i++) {
          const p = eo(seg(t, bTimes[i], bTimes[i] + 1.1));
          $(`s4-lane-fill-${i}`).style.width = `${(p * 100).toFixed(1)}%`;
          $(`s4-lane-card-${i}`).style.opacity = (0.35 + 0.65 * p).toFixed(2);
        }
      }
      const l5 = $("L-scene5");
      const v5 = vis(t, 56.1, 64.75);
      if (applyVis(l5, v5)) {
        l5.style.left = `${(w / 2).toFixed(1)}px`;
        l5.style.top = `${(h / 2).toFixed(1)}px`;
        const fmtTimes = [57.45, 58.3, 59.15];
        for (let i = 0; i < 3; i++) {
          const sPop = spring(t - fmtTimes[i], 22, 0.72);
          const card = $(`s5-fmt-${i}`);
          card.style.opacity = clamp((t - fmtTimes[i]) / 0.15).toFixed(3);
          card.style.transform = `translateY(${((1 - sPop) * 24).toFixed(1)}px) scale(${(0.9 + 0.1 * sPop).toFixed(3)})`;
        }
        const keelTimes = [60.6, 61.5, 62.4];
        for (let i = 0; i < 3; i++) {
          const kp = eo(seg(t, keelTimes[i], keelTimes[i] + 0.5));
          const row = $(`s5-pkg-${i}`);
          row.style.opacity = (0.25 + 0.75 * kp).toFixed(2);
          $(`s5-pkg-chk-${i}`).style.opacity = kp > 0.7 ? "1" : "0.15";
        }
      }
      const l6 = $("L-scene6");
      const v6 = vis(t, 64.75, 77.15);
      if (applyVis(l6, v6)) {
        l6.style.left = `${(w / 2).toFixed(1)}px`;
        l6.style.top = `${(h / 2).toFixed(1)}px`;
        const prTimes = [65.6, 67.2, 68.8];
        for (let i = 0; i < 3; i++) {
          const ok = t >= prTimes[i];
          const row = $(`s6-proof-${i}`);
          row.style.borderColor = ok ? "#10B981" : "#262A36";
          $(`s6-pbadge-${i}`).style.background = ok ? "rgba(16,185,129,0.18)" : "rgba(255,255,255,0.06)";
          $(`s6-pbadge-${i}`).style.color = ok ? "#10B981" : "#71717A";
        }
        const pkt = $("s6-unsafe-pkt");
        const shield = $("s6-shield-bar");
        const rejectBanner = $("s6-reject-banner");
        if (t < 71.3) {
          pkt.style.opacity = "0";
          shield.style.background = "#262A36";
          rejectBanner.style.opacity = "0";
        } else if (t < 73.2) {
          const u = eio(seg(t, 71.3, 73.2));
          pkt.style.opacity = "1";
          pkt.style.transform = `translateX(${lerp(-180, -18, u).toFixed(1)}px)`;
          shield.style.background = "#FF4F1A";
          rejectBanner.style.opacity = "0";
        } else {
          const recoil = spring(t - 73.2, 24, 0.62);
          pkt.style.opacity = clamp(1 - (t - 74.2) * 1.5, 0.25, 1).toFixed(2);
          pkt.style.transform = `translateX(${lerp(-18, -95, recoil).toFixed(1)}px) scale(${lerp(1, 0.92, recoil).toFixed(3)})`;
          shield.style.background = "#FF4F1A";
          shield.style.boxShadow = "0 0 28px rgba(255,79,26,0.75)";
          rejectBanner.style.opacity = eo(seg(t, 73.2, 73.55)).toFixed(3);
        }
      }
      const c = cPath(t);
      const sx = CX + s * (c.x - fx);
      const sy = CY + s * (c.y - fy);
      const cp = cPress(t);
      cur.style.transform = `translate(${(sx - 3).toFixed(1)}px, ${(sy - 3).toFixed(1)}px) scale(${(1 - 0.14 * cp).toFixed(3)})`;
    } else {
      world.style.display = "none";
      cur.style.display = "none";
    }
    const s7Layer = $("scene7-overlay");
    if (t >= 77.1) {
      s7Layer.style.display = "";
      const tripletBox = $("s7-triplet");
      const endCard = $("s7-endcard");
      if (t < 81.15) {
        tripletBox.style.display = "";
        endCard.style.display = "none";
        let word = "L++";
        let sub = "SYSTEMS COMPILER \xB7 v1.0";
        let bg = "#0B0B10";
        let fg = "#EDEBE4";
        let ghostOp = "0.07";
        let tLocal = t - 77.1;
        if (t >= 78.48 && t < 79.36) {
          word = "WRITE IT.";
          sub = "CLEAN INDENTED SYNTAX \xB7 := TYPE INFERENCE";
          bg = "#EDEBE4";
          fg = "#0B0B10";
          ghostOp = "0.08";
          tLocal = t - 78.48;
        } else if (t >= 79.36 && t < 80.22) {
          word = "PROVE IT.";
          sub = "COMPILE-TIME ARC \xB7 E4402 INDEPENDENT VERIFIER";
          bg = "#2F47FF";
          fg = "#FFFFFF";
          ghostOp = "0.12";
          tLocal = t - 79.36;
        } else if (t >= 80.22) {
          word = "SHIP IT.";
          sub = "MILLISECOND CRANELIFT \xB7 DIRECT ELF / PE / MACH-O";
          bg = "#FF4F1A";
          fg = "#0B0B10";
          ghostOp = "0.12";
          tLocal = t - 80.22;
        }
        tripletBox.style.background = bg;
        tripletBox.style.color = fg;
        const slamS = 0.84 + 0.16 * spring(tLocal, 28, 0.66);
        const mainWord = $("s7-triplet-word");
        setText(mainWord, word);
        mainWord.style.transform = `scale(${slamS.toFixed(4)})`;
        setText($("s7-triplet-sub"), sub);
        const ghost = $("s7-triplet-ghost");
        setText(ghost, `${word}   ${word}`);
        ghost.style.opacity = ghostOp;
      } else {
        tripletBox.style.display = "none";
        endCard.style.display = "";
        const ecVis = vis(t, 81.15, null, { din: 0.02, lin: 0.35 });
        applyVis($("s7-end-inner"), ecVis);
        const cmdFull = "lpp main.lpp --run   //   github.com/samarnever-droid/lplusplus";
        const nChars = Math.floor(lerp(0, cmdFull.length, clamp((t - 81.45) / 1.9)));
        setText($("s7-end-cli"), cmdFull.slice(0, nChars));
      }
    } else {
      s7Layer.style.display = "none";
    }
    let curScene = SCENES[0];
    for (const sc of SCENES) {
      if (t >= sc.start && t <= sc.end) {
        curScene = sc;
        break;
      }
    }
    const hudDarkText = isLightStage && t < 77.1;
    const hudColor = hudDarkText ? "rgba(11, 11, 16, 0.58)" : "rgba(237, 235, 228, 0.58)";
    const hudEl = $("hud-layer");
    hudEl.style.color = hudColor;
    setText($("hud-scene-idx"), curScene.tag);
    setText($("hud-category"), curScene.category);
    const mins = Math.floor(t / 60);
    const secs = Math.floor(t % 60);
    const frames = Math.floor(t % 1 * 30);
    setText(
      $("hud-timecode"),
      `${String(mins).padStart(2, "0")}:${String(secs).padStart(2, "0")}:${String(frames).padStart(2, "0")}`
    );
    const capPill = $("hud-caption-pill");
    const activeCue = VO_CUES.find((c) => t >= c.start - 0.08 && t <= c.end + 0.18);
    if (activeCue) {
      capPill.style.opacity = "1";
      const words = activeCue.text.split(" ");
      const prog = clamp((t - activeCue.start) / Math.max(0.1, activeCue.end - activeCue.start));
      const activeWordIdx = Math.min(words.length - 1, Math.floor(prog * words.length));
      capPill.innerHTML = words.map(
        (w, idx) => idx === activeWordIdx ? `<span class="cap-w active">${escapeHtml(w)}</span>` : idx < activeWordIdx ? `<span class="cap-w spoken">${escapeHtml(w)}</span>` : `<span class="cap-w">${escapeHtml(w)}</span>`
      ).join(" ");
    } else {
      capPill.style.opacity = "0";
    }
  }
  window.seek = seek;
  function boot() {
    initDom();
    const isRender = location.search.includes("render");
    document.body.classList.add(isRender ? "render" : "preview");
    seek(0);
    if (!isRender) {
      const wr = $("wrap");
      const fit = () => {
        const k = Math.min(window.innerWidth / W, (window.innerHeight - 56) / H);
        wr.style.transform = `translate(-50%, -50%) scale(${k.toFixed(4)})`;
      };
      fit();
      window.addEventListener("resize", fit);
      const audio = document.getElementById("master-audio");
      const playBtn = document.getElementById("ctrl-play");
      const scrub = document.getElementById("ctrl-scrub");
      const timeLabel = document.getElementById("ctrl-time");
      let playing = false;
      let fallbackStart = performance.now();
      let fallbackOffset = 0;
      const getNow = () => {
        if (audio && !audio.paused && !isNaN(audio.currentTime)) {
          return audio.currentTime % VIDEO_DURATION;
        }
        if (playing) {
          return (fallbackOffset + (performance.now() - fallbackStart) / 1e3) % VIDEO_DURATION;
        }
        return fallbackOffset;
      };
      const togglePlay = () => {
        if (!playing) {
          playing = true;
          fallbackStart = performance.now();
          if (audio) {
            audio.currentTime = fallbackOffset;
            audio.play().catch(() => {
            });
          }
          if (playBtn) playBtn.textContent = "Pause";
        } else {
          fallbackOffset = getNow();
          playing = false;
          if (audio) audio.pause();
          if (playBtn) playBtn.textContent = "Play";
        }
      };
      if (playBtn) playBtn.addEventListener("click", togglePlay);
      if (scrub) {
        scrub.addEventListener("input", () => {
          const val = parseFloat(scrub.value);
          fallbackOffset = val;
          if (audio) audio.currentTime = val;
          seek(val);
        });
      }
      const loop = () => {
        const curT = getNow();
        seek(curT);
        if (scrub && document.activeElement !== scrub) scrub.value = curT.toFixed(2);
        if (timeLabel) timeLabel.textContent = `${curT.toFixed(1)}s / ${VIDEO_DURATION.toFixed(1)}s`;
        requestAnimationFrame(loop);
      };
      requestAnimationFrame(loop);
    }
  }
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", boot);
  } else {
    boot();
  }
})();
