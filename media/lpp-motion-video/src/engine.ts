// Barty-Bart Motion-Graphics Engine in TypeScript
// Pure function of time t: every transform, spring, opacity, and text state is computed deterministically from t.

export type SpringPreset = [number, number]; // [stiffness w, damping z]
export type TrackKey = [number, number, SpringPreset?];
export type ColorKey = [number, string, SpringPreset?];
export type EasedKey = [number, number, ((x: number) => number)?];
export type CursorKey = [number, number, number];

export const MORPH: SpringPreset = [15, 0.84];
export const FAST: SpringPreset = [27, 0.86];
export const SLOW: SpringPreset = [12.5, 0.90];
export const SOFT: SpringPreset = [10, 0.95];
export const CAM: SpringPreset = [7.5, 1.0];
export const SLAM: SpringPreset = [22, 0.68];
export const INSTANT: SpringPreset = [Infinity, 1.0];

export function clamp(x: number, a = 0, b = 1): number {
  return Math.min(b, Math.max(a, x));
}

export function lerp(a: number, b: number, u: number): number {
  return a + (b - a) * u;
}

export function seg(t: number, a: number, b: number): number {
  if (Math.abs(b - a) < 1e-6) return t >= b ? 1 : 0;
  return clamp((t - a) / (b - a), 0, 1);
}

export function eo(x: number): number {
  const u = clamp(x);
  return 1 - Math.pow(1 - u, 3);
}

export function eio(x: number): number {
  const u = clamp(x);
  return u * u * u * (u * (6 * u - 15) + 10);
}

export function easeInOutCubic(x: number): number {
  const u = clamp(x);
  return u < 0.5 ? 4 * u * u * u : 1 - Math.pow(-2 * u + 2, 3) / 2;
}

// Closed-form damped harmonic spring step response 0 -> 1 (zero initial velocity)
export function spring(tau: number, w = 14, z = 0.82): number {
  if (tau <= 0) return 0;
  if (!isFinite(w)) return 1;
  if (z < 1) {
    const wd = w * Math.sqrt(1 - z * z);
    return 1 - Math.exp(-z * w * tau) * (Math.cos(wd * tau) + ((z * w) / wd) * Math.sin(wd * tau));
  }
  if (z > 1 + 1e-6) {
    const q = w * Math.sqrt(z * z - 1);
    const r1 = -z * w + q;
    const r2 = -z * w - q;
    return 1 - (r2 * Math.exp(r1 * tau) - r1 * Math.exp(r2 * tau)) / (r2 - r1);
  }
  return 1 - Math.exp(-w * tau) * (1 + w * tau);
}

// Multi-target spring track: sum of one closed-form spring per target change
export function track(v0: number, keys: TrackKey[], def: SpringPreset = MORPH): (t: number) => number {
  let prev = v0;
  const ks: Array<[number, number, number, number]> = [];
  for (const k of keys) {
    const d = k[1] - prev;
    prev = k[1];
    const sp = k[2] || def;
    if (d !== 0) ks.push([k[0], d, sp[0], sp[1]]);
  }
  return (t: number) => {
    let v = v0;
    for (const k of ks) {
      if (t > k[0]) v += k[1] * spring(t - k[0], k[2], k[3]);
    }
    return v;
  };
}

export function hexToRgb(h: string): [number, number, number] {
  const clean = h.replace('#', '');
  return [
    parseInt(clean.slice(0, 2), 16),
    parseInt(clean.slice(2, 4), 16),
    parseInt(clean.slice(4, 6), 16),
  ];
}

// Closed-form spring RGB color track
export function ctrack(h0: string, keys: ColorKey[], def: SpringPreset = [20, 1]): (t: number) => string {
  const c0 = hexToRgb(h0);
  const tr = [0, 1, 2].map((i) =>
    track(
      c0[i],
      keys.map((k) => [k[0], hexToRgb(k[1])[i], k[2]]),
      def
    )
  );
  return (t: number) =>
    `rgb(${tr.map((f) => Math.round(clamp(f(t), 0, 255))).join(',')})`;
}

// Eased keyframe track (ideal for smooth camera trajectories)
export function kf(keys: EasedKey[]): (t: number) => number {
  return (t: number) => {
    if (t <= keys[0][0]) return keys[0][1];
    for (let i = 1; i < keys.length; i++) {
      const [a, va] = keys[i - 1];
      const [b, vb, fn = easeInOutCubic] = keys[i];
      if (t <= b) return lerp(va, vb, fn((t - a) / (b - a)));
    }
    return keys[keys.length - 1][1];
  };
}

export interface VisResult {
  o: number;
  blur: number;
  s: number;
  a: number;
  b: number;
}

// Content swap helper: outgoing blurs out fast; incoming waits din then blurs/scales in
export function vis(
  t: number,
  tin: number | null,
  tout: number | null,
  opts: { din?: number; lin?: number; lout?: number; blur?: number } = {}
): VisResult {
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
    b,
  };
}

export function applyVis(el: HTMLElement | null, v: VisResult, extraTransform = ''): boolean {
  if (!el) return false;
  if (v.o < 0.002) {
    if (el.style.display !== 'none') el.style.display = 'none';
    return false;
  }
  if (el.style.display === 'none') el.style.display = '';
  el.style.opacity = v.o.toFixed(4);
  el.style.filter = v.blur > 0.08 ? `blur(${v.blur.toFixed(2)}px)` : 'none';
  el.style.transform = `scale(${v.s.toFixed(4)})${extraTransform ? ' ' + extraTransform : ''}`;
  return true;
}

export function setText(el: HTMLElement | null, s: string): void {
  if (!el) return;
  const anyEl = el as unknown as { _t?: string };
  if (anyEl._t !== s) {
    el.textContent = s;
    anyEl._t = s;
  }
}

// Seeded Mulberry32 PRNG — never Math.random()
export function rng(seed: number): () => number {
  let s = seed | 0;
  return () => {
    s = (s + 0x6d2b79f5) | 0;
    let t = Math.imul(s ^ (s >>> 15), 1 | s);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// Cursor path with natural human arc between keyframes
export function cursorPath(keys: CursorKey[]): (t: number) => { x: number; y: number } {
  return (t: number) => {
    if (t <= keys[0][0]) return { x: keys[0][1], y: keys[0][2] };
    const last = keys[keys.length - 1];
    if (t >= last[0]) return { x: last[1], y: last[2] };
    let i = 0;
    while (i + 1 < keys.length && t >= keys[i + 1][0]) i++;
    const [t0, x0, y0] = keys[i];
    const [t1, x1, y1] = keys[i + 1];
    const u = eio((t - t0) / Math.max(0.0001, t1 - t0));
    const dx = x1 - x0;
    const dy = y1 - y0;
    const arc = Math.sin(Math.PI * u) * 0.06;
    return {
      x: x0 + dx * u - dy * arc,
      y: y0 + dy * u + dx * arc,
    };
  };
}

export function cursorPresses(clicks: number[] = [], drags: Array<[number, number]> = []): (t: number) => number {
  const k: TrackKey[] = [];
  for (const c of clicks) {
    k.push([c - 0.07, 1, [45, 1]], [c + 0.04, 0, [22, 0.72]]);
  }
  for (const [a, b] of drags) {
    k.push([a - 0.04, 1, [45, 1]], [b, 0, [22, 0.72]]);
  }
  k.sort((a, b) => a[0] - b[0]);
  return track(0, k);
}

// Stroke-normalized 24-grid vector icons (Lucide ISC compatible paths)
export const IC: Record<string, string[]> = {
  arrow: ['M5 12h14', 'M13 5l7 7-7 7'],
  check: ['M20 6 9 17l-5-5'],
  x: ['M18 6 6 18', 'M6 6l12 12'],
  plus: ['M5 12h14', 'M12 5v14'],
  terminal: ['m4 17 6-6-6-6', 'M12 19h8'],
  file: ['M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z', 'M14 2v4a2 2 0 0 0 2 2h4', 'M8 13h8', 'M8 17h5'],
  folder: ['M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z'],
  clock: ['M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20Z', 'M12 6v6l4 2'],
  chip: ['M6 4h12a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2Z', 'M9 9h6v6H9z', 'M9 1v3', 'M15 1v3', 'M9 20v3', 'M15 20v3', 'M20 9h3', 'M20 14h3', 'M1 9h3', 'M1 14h3'],
  sparkle: ['M9.94 15.5A2 2 0 0 0 8.5 14.06l-6.14-1.58a.5.5 0 0 1 0-.96L8.5 9.94A2 2 0 0 0 9.94 8.5l1.58-6.14a.5.5 0 0 1 .96 0L14.06 8.5A2 2 0 0 0 15.5 9.94l6.14 1.58a.5.5 0 0 1 0 .96L15.5 14.06a2 2 0 0 0-1.44 1.44l-1.58 6.14a.5.5 0 0 1-.96 0z'],
  shield: ['M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z'],
  package: ['m7.5 4.27 9 5.15', 'M21 8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16Z', 'm3.3 7 8.7 5 8.7-5', 'M12 22V12'],
  layers: ['m12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83Z', 'm22 17.65-9.17 4.16a2 2 0 0 1-1.66 0L2 17.65', 'm22 12.65-9.17 4.16a2 2 0 0 1-1.66 0L2 12.65'],
};

export function iconSvg(name: string, size: number, color: string, sw = 2.2): string {
  const paths = IC[name] || IC.sparkle;
  const strokeW = ((sw * 24) / size).toFixed(3);
  return `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="${color}" stroke-width="${strokeW}" stroke-linecap="round" stroke-linejoin="round">${paths
    .map((d) => `<path d="${d}"/>`)
    .join('')}</svg>`;
}
