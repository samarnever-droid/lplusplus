import {
  MORPH,
  FAST,
  SLOW,
  CAM,
  SLAM,
  clamp,
  lerp,
  seg,
  eo,
  eio,
  spring,
  track,
  ctrack,
  kf,
  vis,
  applyVis,
  setText,
  cursorPath,
  cursorPresses,
  iconSvg,
} from './engine';
import { VIDEO_DURATION, VO_CUES, SCENES, LPP_CODE_LINES } from './data';

declare global {
  interface Window {
    VIDEO: { w: number; h: number; duration: number };
    seek: (t: number) => void;
    DURATION: number;
  }
}

const W = 1920;
const H = 1080;
const CX = W / 2;
const CY = H / 2;

window.VIDEO = { w: W, h: H, duration: VIDEO_DURATION };
window.DURATION = VIDEO_DURATION;

const $ = (id: string): HTMLElement => document.getElementById(id)!;

// ============================================================================
// 1. ONE-SHAPE MORPH STATES (from Scene 02 at 7.55s through Scene 06 at 77.15s)
// ============================================================================
interface ShapeState {
  w: number;
  h: number;
  r: number;
  bg: string;
  cam: number;
  border: string;
}

const SH: Record<string, ShapeState> = {
  seed:    { w: 76,   h: 76,  r: 38, bg: '#FF4F1A', cam: 1.0,  border: '#FF4F1A' },
  editor:  { w: 1180, h: 620, r: 26, bg: '#111318', cam: 1.18, border: '#262934' },
  build:   { w: 1260, h: 650, r: 28, bg: '#111318', cam: 1.14, border: '#FF4F1A' },
  memClean:{ w: 1080, h: 550, r: 30, bg: '#FFFFFF', cam: 1.24, border: '#E2E0D8' },
  arcWork: { w: 1320, h: 670, r: 28, bg: '#111318', cam: 1.12, border: '#2A2E3D' },
  pipe:    { w: 1360, h: 660, r: 30, bg: '#0E1015', cam: 1.10, border: '#252936' },
  linkKeel:{ w: 1320, h: 630, r: 28, bg: '#FFFFFF', cam: 1.14, border: '#DFDDD5' },
  safety:  { w: 1320, h: 660, r: 28, bg: '#111318', cam: 1.13, border: '#2A2E3D' },
  expand:  { w: 1960, h: 1120,r: 0,  bg: '#0B0B10', cam: 1.0,  border: '#0B0B10' },
};

const SEQ: Array<[number, keyof typeof SH]> = [
  [7.48,  'editor'],
  [17.20, 'build'],
  [22.65, 'memClean'],
  [28.95, 'arcWork'],
  [39.60, 'pipe'],
  [56.10, 'linkKeel'],
  [64.75, 'safety'],
  [77.10, 'expand'],
];

const shW = track(SH.seed.w, SEQ.map(([t, k]) => [t, SH[k].w]), MORPH);
const shH = track(SH.seed.h, SEQ.map(([t, k]) => [t, SH[k].h]), MORPH);
const shR = track(SH.seed.r, SEQ.map(([t, k]) => [t, SH[k].r]), MORPH);
const shCam = track(SH.seed.cam, SEQ.map(([t, k]) => [t, SH[k].cam]), CAM);
const shBG = ctrack(SH.seed.bg, SEQ.map(([t, k]) => [t, SH[k].bg]), [18, 0.95]);
const shBorder = ctrack(SH.seed.border, SEQ.map(([t, k]) => [t, SH[k].border]), [18, 0.95]);

// Stage background color track across the full 84.5s trailer
const stageBG = ctrack(
  '#0B0B10',
  [
    [1.85,  '#FF4F1A', [24, 0.92]],
    [2.55,  '#EDEBE4', [18, 0.94]],
    [7.55,  '#E9E7E2', [14, 0.95]],
    [28.95, '#E4E2DC', [14, 0.95]],
    [39.60, '#E9E7E2', [14, 0.95]],
    [77.10, '#0B0B10', [24, 0.95]],
  ]
);

// Camera focus drift (world coordinates) for dynamic framing inside scenes
const camFx = kf([
  [0, 0],
  [8.2, -90],
  [14.5, -40],
  [17.3, 60],
  [22.6, 0],
  [29.2, -50],
  [34.0, 60],
  [39.6, 0],
  [41.0, -140],
  [47.2, 120],
  [49.0, 0],
  [56.1, -90],
  [60.5, 90],
  [64.8, -80],
  [71.2, 85],
  [76.8, 0],
]);

const camFy = kf([
  [0, 0],
  [8.5, -20],
  [15.0, 25],
  [18.0, 0],
  [29.5, -20],
  [34.5, 20],
  [40.0, -35],
  [48.5, 35],
  [56.1, 0],
  [77.0, 0],
]);

// Shape press bounce on key interactions
const shapePress = track(0, [
  [18.10 - 0.06, 1, [45, 1]],
  [18.10 + 0.04, 0, [22, 0.72]],
  [24.40 - 0.06, 1, [45, 1]],
  [24.40 + 0.04, 0, [22, 0.72]],
  [61.10 - 0.06, 1, [45, 1]],
  [61.10 + 0.04, 0, [22, 0.72]],
  [73.20 - 0.06, 1.3, [45, 1]],
  [73.20 + 0.05, 0, [20, 0.65]],
]);

// ============================================================================
// 2. CURSOR KEYFRAMES & CLICKS (World Coordinates)
// ============================================================================
const cPath = cursorPath([
  [0.0,   520,  340],
  [8.0,   380,  220],
  [12.0,  240,  110],
  [16.8,  290,  180],
  [17.95, 310, -225], // Move to "lpp main.lpp --run" button
  [19.5,  390,   60],
  [22.8, -320,  -95], // Scene 3A: start of hazard strike
  [24.4,  280,  -95],
  [25.8,  280,   15],
  [27.2,  280,  125],
  [29.4, -380,   25], // Scene 3B: Placement tabs
  [31.8, -380,   95],
  [34.2, -380,  165],
  [36.0,  290,  120], // Cycle collector node
  [40.2, -460, -145], // Scene 4: Pipeline stations
  [42.2, -450, -110],
  [43.4, -155, -110],
  [44.7,  145, -110],
  [46.1,  445, -110],
  [49.0,  340,   55], // Cranelift lane
  [51.5,  340,  140], // LLVM lane
  [52.8,  340,  225], // WASM lane
  [57.4, -310,  -40], // Scene 5: ELF / PE / Mach-O
  [58.4, -310,   55],
  [59.3, -310,  150],
  [61.0,  310, -175], // Keel workspace build button
  [63.0,  280,   95],
  [65.5, -290,  -40], // Scene 6: Ownership proof checks
  [68.0, -290,   60],
  [71.5,  290,   10], // Rejection shield
  [76.8,  560,  360],
]);

const cPress = cursorPresses(
  [18.10, 24.40, 25.80, 27.20, 31.80, 34.20, 42.20, 43.40, 44.70, 46.10, 57.60, 58.40, 59.20, 61.10, 66.20, 68.10, 73.20],
  [[24.20, 27.30]]
);

// ============================================================================
// 3. SCENE-SPECIFIC LIQUID SPRINGS
// ============================================================================
// Scene 2: Liquid build progress bar (FAST leading edge, SLOW trailing edge)
const buildLead = track(0, [[18.12, 100, FAST]]);
const buildTrail = track(0, [[18.12, 100, SLOW]]);

// Scene 3B: Liquid 3-tier placement selector pill (y offset: 0 -> 72 -> 144)
const tierTop = track(0, [
  [29.8, 0, FAST],
  [31.8, 72, SLOW],
  [34.2, 144, SLOW],
]);
const tierBot = track(58, [
  [29.8, 58, SLOW],
  [31.8, 130, FAST],
  [34.2, 202, FAST],
]);

// Scene 4: Hero orb X position across the 4 compiler stages
const pipeHeroX = track(-450, [
  [41.8, -450, FAST],
  [43.2, -150, MORPH],
  [44.5,  150, MORPH],
  [45.9,  450, MORPH],
]);

// ============================================================================
// 4. INITIALIZE DOM STRUCTURE ONCE AT LOAD
// ============================================================================
function initDom(): void {
  // Inject Lucide/Geist stroke icons into placeholders
  document.querySelectorAll<HTMLElement>('[data-icon]').forEach((el) => {
    const name = el.dataset.icon || 'sparkle';
    const size = Number(el.dataset.size || 20);
    const color = el.dataset.color || 'currentColor';
    const sw = Number(el.dataset.sw || 2.2);
    el.innerHTML = iconSvg(name, size, color, sw);
  });

  // Populate static code lines container in Scene 2 for fast character slicing
  const codeBox = $('s2-code-lines');
  codeBox.innerHTML = LPP_CODE_LINES.map((line, idx) => {
    const pad = '&nbsp;'.repeat(line.indent * 4);
    return `<div class="code-line" id="s2-line-${idx}"><span class="ln">${String(idx + 1).padStart(2, '0')}</span><span class="lc">${pad}<span id="s2-tok-${idx}"></span></span></div>`;
  }).join('');
}

// Helper to render typed tokens up to `charBudget` characters
function updateTypedCode(charBudget: number, t: number): void {
  let rem = Math.max(0, Math.floor(charBudget));
  let caretPlaced = false;
  const caretOn = Math.floor(t * 3) % 2 === 0;

  for (let i = 0; i < LPP_CODE_LINES.length; i++) {
    const rowEl = $(`s2-line-${i}`);
    const tokContainer = $(`s2-tok-${i}`);
    const tokens = LPP_CODE_LINES[i].tokens;
    const lineLen = tokens.reduce((acc, tk) => acc + tk[0].length, 0);

    if (rem <= 0 && i > 0 && !caretPlaced) {
      rowEl.style.opacity = '0.18';
      setText(tokContainer, '');
      continue;
    }

    rowEl.style.opacity = '1';
    if (rem >= lineLen) {
      rem -= lineLen;
      const html = tokens.map(([txt, cls]) => `<span class="tk-${cls}">${escapeHtml(txt)}</span>`).join('');
      if (tokContainer.dataset.full !== '1') {
        tokContainer.innerHTML = html;
        tokContainer.dataset.full = '1';
      }
    } else {
      tokContainer.dataset.full = '0';
      let left = rem;
      let html = '';
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

function escapeHtml(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

// ============================================================================
// 5. PURE STATELESS SEEK(T) FUNCTION
// ============================================================================
export function seek(rawT: number): void {
  const t = clamp(rawT, 0, VIDEO_DURATION);

  // 5.1 Stage base background + subtle dot grid opacity
  const stage = $('stage');
  stage.style.background = stageBG(t);
  const gridEl = $('bg-grid');
  const isLightStage = t >= 2.35 && t < 77.10;
  gridEl.style.opacity = isLightStage ? '0.28' : '0.16';
  gridEl.style.backgroundImage = isLightStage
    ? 'radial-gradient(#111318 1.25px, transparent 1.25px)'
    : 'radial-gradient(#FFFFFF 1.25px, transparent 1.25px)';

  // --------------------------------------------------------------------------
  // SCENE 01 (0.00s - 7.75s): SPARK OPEN -> FLOOD -> BRAND SLAM -> SEED BRIDGE
  // --------------------------------------------------------------------------
  const s1Layer = $('scene1-overlay');
  if (t < 7.85) {
    s1Layer.style.display = '';
    // Spark & horizontal line (0.00s - 2.40s)
    const sparkWrap = $('s1-spark-wrap');
    const sparkP = spring(t - 0.12, 18, 0.72);
    const lineExpand = eo(seg(t, 0.45, 1.45));
    const floodExpand = eio(seg(t, 1.80, 2.45));
    const lineW = lerp(14, 1560, lineExpand);
    const lineH = lerp(4, 1120, floodExpand);
    sparkWrap.style.opacity = t < 2.55 ? '1' : '0';
    const sparkBar = $('s1-spark-bar');
    sparkBar.style.width = `${lineW.toFixed(1)}px`;
    sparkBar.style.height = `${lineH.toFixed(1)}px`;
    sparkBar.style.borderRadius = `${lerp(99, 0, floodExpand).toFixed(1)}px`;
    sparkBar.style.transform = `translate(-50%, -50%) scale(${sparkP.toFixed(3)})`;

    // Mono label above spark line
    const sparkLabel = $('s1-spark-label');
    const slVis = vis(t, 0.35, 1.82, { din: 0.02, lin: 0.2, lout: 0.12 });
    applyVis(sparkLabel, slVis);

    // Phase B (2.30s - 4.35s): "EVERY SECOND WAITING IS A SECOND NOT BUILDING"
    const waitBlock = $('s1-wait-block');
    const wbVis = vis(t, 2.25, 4.35, { din: 0.04, lin: 0.24, lout: 0.15 });
    if (applyVis(waitBlock, wbVis)) {
      const waitSec = (1.4 + (t - 2.25) * 3.8).toFixed(1);
      setText($('s1-wait-timer'), `compiling... ${waitSec}s`);
      const strikeW = eo(seg(t, 3.25, 3.85)) * 100;
      $('s1-wait-strike').style.width = `${strikeW.toFixed(1)}%`;
    }

    // Phase C (4.35s - 7.75s): BRAND SLAM "MEET L++"
    const brandBlock = $('s1-brand-block');
    const bbVis = vis(t, 4.32, 7.48, { din: 0.04, lin: 0.24, lout: 0.16 });
    if (applyVis(brandBlock, bbVis)) {
      const slamScale = 0.82 + 0.18 * spring(t - 4.35, 22, 0.68);
      const wght = Math.round(lerp(300, 900, eo(seg(t, 4.35, 5.10))));
      const lppTitle = $('s1-lpp-title');
      lppTitle.style.fontVariationSettings = `'wght' ${wght}`;
      lppTitle.style.transform = `scale(${slamScale.toFixed(4)})`;
    }
  } else {
    s1Layer.style.display = 'none';
  }

  // --------------------------------------------------------------------------
  // ONE-SHAPE CONTINUOUS MORPH (7.35s - 77.25s)
  // --------------------------------------------------------------------------
  const world = $('world');
  const shape = $('shape');
  const cur = $('cursor');

  if (t >= 7.35 && t < 77.25) {
    world.style.display = '';
    cur.style.display = '';

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

    // Update Layer 2: SYNTAX & MILLISECOND BUILD (7.55s - 22.65s)
    const l2 = $('L-scene2');
    const v2 = vis(t, 7.55, 22.65);
    if (applyVis(l2, v2)) {
      l2.style.left = `${(w / 2).toFixed(1)}px`;
      l2.style.top = `${(h / 2).toFixed(1)}px`;

      // Type code from 8.20s to 16.40s (~265 chars total)
      const chars = Math.floor(lerp(0, 275, clamp((t - 8.20) / 8.0)));
      updateTypedCode(chars, t);

      // Build button state (clicked at 18.10s)
      const btn = $('s2-build-btn');
      const clicked = t >= 18.10;
      btn.style.background = clicked ? '#10B981' : '#FF4F1A';
      setText($('s2-build-label'), clicked ? 'Built · main.exe ✓' : 'lpp main.lpp --run');

      // Liquid build bar & millisecond readout
      const lead = buildLead(t);
      const trail = buildTrail(t);
      const barFill = $('s2-bar-fill');
      barFill.style.width = `${clamp(lead, 0, 100).toFixed(1)}%`;
      barFill.style.opacity = t >= 18.05 ? '1' : '0.2';

      const msVal = t < 18.10 ? '0.0' : (3.4 * eo(seg(t, 18.10, 18.85))).toFixed(1);
      setText($('s2-ms-num'), `${msVal} ms`);

      // Native binary output card pop-in at 18.55s
      const exeCard = $('s2-exe-card');
      const exeVis = vis(t, 18.50, 22.60, { din: 0.04, lin: 0.22, lout: 0.12 });
      applyVis(exeCard, exeVis);
    }

    // Update Layer 3A: MEMORY CONTRACT — NO MANUAL FREES (22.65s - 28.95s)
    const l3a = $('L-scene3a');
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
          badge.textContent = 'ELIMINATED · COMPILE TIME';
          badge.className = 'hz-badge done';
        } else {
          badge.textContent = 'MANUAL HAZARD';
          badge.className = 'hz-badge warn';
        }
      }
    }

    // Update Layer 3B: 24-BYTE ARC HEADER + 3-TIER PLACEMENT + CYCLE COLLECTOR (28.95s - 39.60s)
    const l3b = $('L-scene3b');
    const v3b = vis(t, 28.95, 39.60);
    if (applyVis(l3b, v3b)) {
      l3b.style.left = `${(w / 2).toFixed(1)}px`;
      l3b.style.top = `${(h / 2).toFixed(1)}px`;

      // Liquid placement pill (stretches between top and bottom springs)
      const topY = tierTop(t);
      const botY = tierBot(t);
      const pill = $('s3b-tier-pill');
      pill.style.top = `${topY.toFixed(1)}px`;
      pill.style.height = `${Math.max(44, botY - topY).toFixed(1)}px`;

      // Cycle collector trial deletion states (34.5s -> 38.8s)
      const cycPhase = t < 34.8 ? 0 : t < 36.4 ? 1 : t < 37.6 ? 2 : 3;
      const phaseNames = [
        '1 · ACTIVE CYCLE (Shared SCC)',
        '2 · MARK GRAY (Trial Decrement)',
        '3 · SCAN WHITE (0 External Roots)',
        '4 · RECLAIMED (0 Leaks · E4304 PASS)',
      ];
      const phaseColors = ['#FF4F1A', '#F59E0B', '#38BDF8', '#10B981'];
      const statusEl = $('s3b-cycle-status');
      setText(statusEl, phaseNames[cycPhase]);
      statusEl.style.color = phaseColors[cycPhase];

      const rcVals = cycPhase === 0 ? ['rc: 2', 'rc: 2', 'rc: 2'] : cycPhase === 1 ? ['rc: 1 (gray)', 'rc: 1 (gray)', 'rc: 1 (gray)'] : cycPhase === 2 ? ['rc: 0 (white)', 'rc: 0 (white)', 'rc: 0 (white)'] : ['FREED ✓', 'FREED ✓', 'FREED ✓'];
      for (let i = 0; i < 3; i++) {
        const nd = $(`s3b-node-${i}`);
        setText($(`s3b-rc-${i}`), rcVals[i]);
        nd.style.borderColor = phaseColors[cycPhase];
        const shrink = cycPhase === 3 ? 0.88 : 1.0;
        nd.style.transform = `scale(${shrink})`;
        nd.style.opacity = cycPhase === 3 ? '0.65' : '1';
      }
    }

    // Update Layer 4: 15-CRATE PIPELINE & 3 BACKEND LANES (39.60s - 56.10s)
    const l4 = $('L-scene4');
    const v4 = vis(t, 39.60, 56.10);
    if (applyVis(l4, v4)) {
      l4.style.left = `${(w / 2).toFixed(1)}px`;
      l4.style.top = `${(h / 2).toFixed(1)}px`;

      // Hero orb moving across the 4 pipeline stations
      const hx = pipeHeroX(t);
      const orb = $('s4-hero-orb');
      orb.style.transform = `translate(${hx.toFixed(1)}px, -125px)`;
      orb.style.opacity = t < 48.2 ? '1' : '0.35';

      const stTimes = [41.8, 43.2, 44.5, 45.9];
      for (let i = 0; i < 4; i++) {
        const active = t >= stTimes[i];
        const stEl = $(`s4-st-${i}`);
        const hit = spring(t - stTimes[i], 24, 0.65);
        stEl.style.borderColor = active ? '#FF4F1A' : '#262A36';
        stEl.style.background = active ? '#181C26' : '#12151E';
        stEl.style.transform = `scale(${(1 + 0.04 * (1 - Math.abs(hit - 0.5) * 2) * (active ? 1 : 0)).toFixed(3)})`;
      }

      // 3 Backend lanes filling at 48.6s, 51.3s, 52.6s
      const bTimes = [48.6, 51.3, 52.6];
      for (let i = 0; i < 3; i++) {
        const p = eo(seg(t, bTimes[i], bTimes[i] + 1.1));
        $(`s4-lane-fill-${i}`).style.width = `${(p * 100).toFixed(1)}%`;
        $(`s4-lane-card-${i}`).style.opacity = (0.35 + 0.65 * p).toFixed(2);
      }
    }

    // Update Layer 5: DIRECT LINKER (ELF/PE/Mach-O) & KEEL PM (56.10s - 64.75s)
    const l5 = $('L-scene5');
    const v5 = vis(t, 56.10, 64.75);
    if (applyVis(l5, v5)) {
      l5.style.left = `${(w / 2).toFixed(1)}px`;
      l5.style.top = `${(h / 2).toFixed(1)}px`;

      const fmtTimes = [57.45, 58.30, 59.15];
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
        $(`s5-pkg-chk-${i}`).style.opacity = kp > 0.7 ? '1' : '0.15';
      }
    }

    // Update Layer 6: HONEST SAFETY S2 & E4403 REJECTION GATE (64.75s - 77.15s)
    const l6 = $('L-scene6');
    const v6 = vis(t, 64.75, 77.15);
    if (applyVis(l6, v6)) {
      l6.style.left = `${(w / 2).toFixed(1)}px`;
      l6.style.top = `${(h / 2).toFixed(1)}px`;

      // Left proof checks (65.6s, 67.2s, 68.8s)
      const prTimes = [65.6, 67.2, 68.8];
      for (let i = 0; i < 3; i++) {
        const ok = t >= prTimes[i];
        const row = $(`s6-proof-${i}`);
        row.style.borderColor = ok ? '#10B981' : '#262A36';
        $(`s6-pbadge-${i}`).style.background = ok ? 'rgba(16,185,129,0.18)' : 'rgba(255,255,255,0.06)';
        $(`s6-pbadge-${i}`).style.color = ok ? '#10B981' : '#71717A';
      }

      // Right rejection gate: unsafe packet travels 71.4s -> hits shield at 73.2s -> recoils
      const pkt = $('s6-unsafe-pkt');
      const shield = $('s6-shield-bar');
      const rejectBanner = $('s6-reject-banner');
      if (t < 71.3) {
        pkt.style.opacity = '0';
        shield.style.background = '#262A36';
        rejectBanner.style.opacity = '0';
      } else if (t < 73.20) {
        const u = eio(seg(t, 71.3, 73.20));
        pkt.style.opacity = '1';
        pkt.style.transform = `translateX(${lerp(-180, -18, u).toFixed(1)}px)`;
        shield.style.background = '#FF4F1A';
        rejectBanner.style.opacity = '0';
      } else {
        const recoil = spring(t - 73.20, 24, 0.62);
        pkt.style.opacity = clamp(1 - (t - 74.2) * 1.5, 0.25, 1).toFixed(2);
        pkt.style.transform = `translateX(${lerp(-18, -95, recoil).toFixed(1)}px) scale(${lerp(1, 0.92, recoil).toFixed(3)})`;
        shield.style.background = '#FF4F1A';
        shield.style.boxShadow = '0 0 28px rgba(255,79,26,0.75)';
        rejectBanner.style.opacity = eo(seg(t, 73.20, 73.55)).toFixed(3);
      }
    }

    // Cursor position in screen coordinates
    const c = cPath(t);
    const sx = CX + s * (c.x - fx);
    const sy = CY + s * (c.y - fy);
    const cp = cPress(t);
    cur.style.transform = `translate(${(sx - 3).toFixed(1)}px, ${(sy - 3).toFixed(1)}px) scale(${(1 - 0.14 * cp).toFixed(3)})`;
  } else {
    world.style.display = 'none';
    cur.style.display = 'none';
  }

  // --------------------------------------------------------------------------
  // SCENE 07 (77.10s - 84.50s): BEAT-LOCKED KINETIC TRIPLET -> END CARD
  // --------------------------------------------------------------------------
  const s7Layer = $('scene7-overlay');
  if (t >= 77.10) {
    s7Layer.style.display = '';
    const tripletBox = $('s7-triplet');
    const endCard = $('s7-endcard');

    if (t < 81.15) {
      tripletBox.style.display = '';
      endCard.style.display = 'none';

      let word = 'L++';
      let sub = 'SYSTEMS COMPILER · v1.0';
      let bg = '#0B0B10';
      let fg = '#EDEBE4';
      let ghostOp = '0.07';
      let tLocal = t - 77.10;

      if (t >= 78.48 && t < 79.36) {
        word = 'WRITE IT.';
        sub = 'CLEAN INDENTED SYNTAX · := TYPE INFERENCE';
        bg = '#EDEBE4';
        fg = '#0B0B10';
        ghostOp = '0.08';
        tLocal = t - 78.48;
      } else if (t >= 79.36 && t < 80.22) {
        word = 'PROVE IT.';
        sub = 'COMPILE-TIME ARC · E4402 INDEPENDENT VERIFIER';
        bg = '#2F47FF';
        fg = '#FFFFFF';
        ghostOp = '0.12';
        tLocal = t - 79.36;
      } else if (t >= 80.22) {
        word = 'SHIP IT.';
        sub = 'MILLISECOND CRANELIFT · DIRECT ELF / PE / MACH-O';
        bg = '#FF4F1A';
        fg = '#0B0B10';
        ghostOp = '0.12';
        tLocal = t - 80.22;
      }

      tripletBox.style.background = bg;
      tripletBox.style.color = fg;
      const slamS = 0.84 + 0.16 * spring(tLocal, 28, 0.66);
      const mainWord = $('s7-triplet-word');
      setText(mainWord, word);
      mainWord.style.transform = `scale(${slamS.toFixed(4)})`;
      setText($('s7-triplet-sub'), sub);
      const ghost = $('s7-triplet-ghost');
      setText(ghost, `${word}   ${word}`);
      ghost.style.opacity = ghostOp;
    } else {
      tripletBox.style.display = 'none';
      endCard.style.display = '';
      const ecVis = vis(t, 81.15, null, { din: 0.02, lin: 0.35 });
      applyVis($('s7-end-inner'), ecVis);
      const cmdFull = 'lpp main.lpp --run   //   github.com/samarnever-droid/lplusplus';
      const nChars = Math.floor(lerp(0, cmdFull.length, clamp((t - 81.45) / 1.9)));
      setText($('s7-end-cli'), cmdFull.slice(0, nChars));
    }
  } else {
    s7Layer.style.display = 'none';
  }

  // --------------------------------------------------------------------------
  // 5.2 CAMERA VIEWFINDER HUD & SYNCHRONIZED SPOKEN CAPTION BAR
  // --------------------------------------------------------------------------
  let curScene = SCENES[0];
  for (const sc of SCENES) {
    if (t >= sc.start && t <= sc.end) {
      curScene = sc;
      break;
    }
  }
  const hudDarkText = isLightStage && t < 77.10;
  const hudColor = hudDarkText ? 'rgba(11, 11, 16, 0.58)' : 'rgba(237, 235, 228, 0.58)';
  const hudEl = $('hud-layer');
  hudEl.style.color = hudColor;
  setText($('hud-scene-idx'), curScene.tag);
  setText($('hud-category'), curScene.category);

  // Format MM:SS.FF timecode
  const mins = Math.floor(t / 60);
  const secs = Math.floor(t % 60);
  const frames = Math.floor((t % 1) * 30);
  setText(
    $('hud-timecode'),
    `${String(mins).padStart(2, '0')}:${String(secs).padStart(2, '0')}:${String(frames).padStart(2, '0')}`
  );

  // Active Voiceover Caption Pill (synced to VO_CUES)
  const capPill = $('hud-caption-pill');
  const activeCue = VO_CUES.find((c) => t >= c.start - 0.08 && t <= c.end + 0.18);
  if (activeCue) {
    capPill.style.opacity = '1';
    const words = activeCue.text.split(' ');
    const prog = clamp((t - activeCue.start) / Math.max(0.1, activeCue.end - activeCue.start));
    const activeWordIdx = Math.min(words.length - 1, Math.floor(prog * words.length));
    capPill.innerHTML = words
      .map((w, idx) =>
        idx === activeWordIdx
          ? `<span class="cap-w active">${escapeHtml(w)}</span>`
          : idx < activeWordIdx
          ? `<span class="cap-w spoken">${escapeHtml(w)}</span>`
          : `<span class="cap-w">${escapeHtml(w)}</span>`
      )
      .join(' ');
  } else {
    capPill.style.opacity = '0';
  }
}

window.seek = seek;

// ============================================================================
// 6. BOOTSTRAP & PREVIEW / RENDER MODE
// ============================================================================
function boot(): void {
  initDom();
  const isRender = location.search.includes('render');
  document.body.classList.add(isRender ? 'render' : 'preview');
  seek(0);

  if (!isRender) {
    const wr = $('wrap');
    const fit = () => {
      const k = Math.min(window.innerWidth / W, (window.innerHeight - 56) / H);
      wr.style.transform = `translate(-50%, -50%) scale(${k.toFixed(4)})`;
    };
    fit();
    window.addEventListener('resize', fit);

    const audio = document.getElementById('master-audio') as HTMLAudioElement | null;
    const playBtn = document.getElementById('ctrl-play');
    const scrub = document.getElementById('ctrl-scrub') as HTMLInputElement | null;
    const timeLabel = document.getElementById('ctrl-time');

    let playing = false;
    let fallbackStart = performance.now();
    let fallbackOffset = 0;

    const getNow = (): number => {
      if (audio && !audio.paused && !isNaN(audio.currentTime)) {
        return audio.currentTime % VIDEO_DURATION;
      }
      if (playing) {
        return (fallbackOffset + (performance.now() - fallbackStart) / 1000) % VIDEO_DURATION;
      }
      return fallbackOffset;
    };

    const togglePlay = () => {
      if (!playing) {
        playing = true;
        fallbackStart = performance.now();
        if (audio) {
          audio.currentTime = fallbackOffset;
          audio.play().catch(() => {});
        }
        if (playBtn) playBtn.textContent = 'Pause';
      } else {
        fallbackOffset = getNow();
        playing = false;
        if (audio) audio.pause();
        if (playBtn) playBtn.textContent = 'Play';
      }
    };

    if (playBtn) playBtn.addEventListener('click', togglePlay);
    if (scrub) {
      scrub.addEventListener('input', () => {
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

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', boot);
} else {
  boot();
}
