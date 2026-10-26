import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const esbuild = require('C:/Users/khati/node_modules/esbuild');

const ROOT = path.resolve('c:/Users/khati/lpp/media/lpp-motion-video');
const DIST = path.join(ROOT, 'dist');
fs.mkdirSync(DIST, { recursive: true });

// 1. Bundle TypeScript -> dist/bundle.js
await esbuild.build({
  entryPoints: [path.join(ROOT, 'src/main.ts')],
  bundle: true,
  format: 'iife',
  target: ['es2022'],
  outfile: path.join(DIST, 'bundle.js'),
  logLevel: 'info',
});

const bundleJs = fs.readFileSync(path.join(DIST, 'bundle.js'), 'utf8');
const geistB64 = fs.readFileSync(path.join(ROOT, 'fonts/Geist-Variable.woff2')).toString('base64');
const monoB64 = fs.readFileSync(path.join(ROOT, 'fonts/GeistMono-Medium.woff2')).toString('base64');

const html = `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8" />
<title>L++ — 2D Motion Graphics Trailer</title>
<style>
@font-face {
  font-family: 'Geist';
  src: url(data:font/woff2;base64,${geistB64}) format('woff2');
  font-weight: 100 900;
  font-display: block;
}
@font-face {
  font-family: 'Geist Mono';
  src: url(data:font/woff2;base64,${monoB64}) format('woff2');
  font-weight: 100 900;
  font-display: block;
}

* { box-sizing: border-box; margin: 0; padding: 0; }
html, body {
  margin: 0;
  background: #0B0B10;
  overflow: hidden;
  user-select: none;
  -webkit-font-smoothing: antialiased;
}
body.preview {
  height: 100vh;
  background: #141418;
}
#wrap {
  position: relative;
  width: 1920px;
  height: 1080px;
  transform-origin: 50% 50%;
}
body.preview #wrap {
  position: fixed;
  left: 50%;
  top: calc(50% - 24px);
}
#stage {
  position: relative;
  width: 1920px;
  height: 1080px;
  overflow: hidden;
  font-family: 'Geist', system-ui, sans-serif;
  color: #0B0B0B;
  font-feature-settings: "tnum" 1;
}
.mono {
  font-family: 'Geist Mono', ui-monospace, monospace;
  font-feature-settings: "tnum" 1;
}
#bg-grid {
  position: absolute;
  inset: 0;
  background-size: 36px 36px;
  pointer-events: none;
}
#bg-vignette {
  position: absolute;
  inset: 0;
  background: radial-gradient(circle at 50% 50%, transparent 58%, rgba(0,0,0,0.22) 100%);
  pointer-events: none;
}

/* ==========================================================================
   SCENE 01 OVERLAY (SPARK -> FLOOD -> BRAND SLAM)
   ========================================================================== */
#scene1-overlay {
  position: absolute;
  inset: 0;
  pointer-events: none;
  z-index: 4;
}
#s1-spark-wrap {
  position: absolute;
  inset: 0;
}
#s1-spark-bar {
  position: absolute;
  left: 50%;
  top: 50%;
  background: #FF4F1A;
  transform-origin: 50% 50%;
}
#s1-spark-label {
  position: absolute;
  left: 50%;
  top: calc(50% - 46px);
  transform: translateX(-50%);
  margin-left: -240px;
  font-size: 16px;
  letter-spacing: 0.18em;
  color: #EDEBE4;
  white-space: nowrap;
}
#s1-wait-block {
  position: absolute;
  left: 50%;
  top: 50%;
  width: 1200px;
  margin-left: -600px;
  margin-top: -150px;
  text-align: center;
  color: #0B0B10;
}
.wait-pill {
  display: inline-flex;
  align-items: center;
  gap: 14px;
  padding: 14px 28px;
  border-radius: 999px;
  background: #FFFFFF;
  border: 1.5px solid #D6D3CD;
  font-size: 22px;
  font-weight: 600;
  position: relative;
  margin-bottom: 28px;
}
#s1-wait-strike {
  position: absolute;
  left: 16px;
  top: 50%;
  height: 4px;
  background: #FF4F1A;
  border-radius: 2px;
}
.s1-headline {
  font-size: 86px;
  font-weight: 900;
  letter-spacing: -0.04em;
  line-height: 1.02;
  color: #0B0B10;
}
#s1-brand-block {
  position: absolute;
  left: 50%;
  top: 50%;
  width: 1360px;
  margin-left: -680px;
  margin-top: -210px;
  text-align: center;
  color: #0B0B10;
}
.s1-kicker {
  font-size: 17px;
  font-weight: 700;
  letter-spacing: 0.22em;
  color: #FF4F1A;
  margin-bottom: 14px;
}
#s1-lpp-title {
  font-size: 230px;
  font-weight: 900;
  letter-spacing: -0.055em;
  line-height: 0.94;
  color: #0B0B10;
  display: inline-block;
}
#s1-lpp-title .plus {
  color: #FF4F1A;
}
.s1-sub {
  margin-top: 22px;
  font-size: 30px;
  font-weight: 500;
  color: #3A3A40;
  letter-spacing: -0.01em;
}

/* ==========================================================================
   WORLD & ONE MORPHING SHAPE (#world, #shape, .L)
   ========================================================================== */
#world {
  position: absolute;
  left: 0;
  top: 0;
  transform-origin: 0 0;
  z-index: 5;
}
#shape {
  position: absolute;
  overflow: hidden;
  isolation: isolate;
  border: 2px solid #262934;
  box-shadow: 0 2px 6px rgba(15, 14, 12, 0.08), 0 24px 60px -12px rgba(15, 14, 12, 0.22);
}
.L {
  position: absolute;
  width: 0;
  height: 0;
  transform-origin: 0 0;
}

/* ==========================================================================
   SCENE 02: CODE EDITOR & MILLISECOND BUILD
   ========================================================================== */
.s2-wrap {
  position: absolute;
  left: -580px;
  top: -285px;
  width: 1160px;
  height: 570px;
  display: flex;
  gap: 28px;
  color: #F4F2EE;
}
.s2-editor {
  flex: 1.35;
  background: #181B22;
  border: 1px solid #2A2F3D;
  border-radius: 18px;
  padding: 22px 26px;
  display: flex;
  flex-direction: column;
}
.s2-tabbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding-bottom: 14px;
  margin-bottom: 14px;
  border-bottom: 1px solid #262B38;
}
.s2-dots { display: flex; gap: 8px; }
.s2-dots i { width: 11px; height: 11px; border-radius: 50%; display: block; }
.code-line {
  display: flex;
  align-items: center;
  font-family: 'Geist Mono', monospace;
  font-size: 19px;
  line-height: 1.62;
  white-space: nowrap;
}
.code-line .ln {
  width: 38px;
  color: #525866;
  font-size: 15px;
}
.tk-kw { color: #FF6B3D; font-weight: 600; }
.tk-ty { color: #60A5FA; font-weight: 600; }
.tk-fn { color: #FBBF24; }
.tk-st { color: #34D399; }
.tk-nu { color: #C084FC; }
.tk-cm { color: #6B7280; }
.tk-id { color: #F4F2EE; }
.tk-op { color: #9CA3AF; }
.code-caret {
  display: inline-block;
  width: 9px;
  height: 21px;
  background: #FF4F1A;
  vertical-align: -3px;
  margin-left: 2px;
}
.s2-right {
  flex: 0.95;
  display: flex;
  flex-direction: column;
  gap: 18px;
}
#s2-build-btn {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 18px 24px;
  border-radius: 16px;
  background: #FF4F1A;
  color: #FFFFFF;
  font-size: 20px;
  font-weight: 700;
}
.s2-telemetry {
  background: #181B22;
  border: 1px solid #2A2F3D;
  border-radius: 18px;
  padding: 22px 24px;
}
.s2-ms-label {
  font-size: 14px;
  letter-spacing: 0.14em;
  color: #9CA3AF;
  margin-bottom: 6px;
}
#s2-ms-num {
  font-size: 64px;
  font-weight: 900;
  color: #F4F2EE;
  letter-spacing: -0.04em;
}
.s2-bar-track {
  margin-top: 14px;
  height: 12px;
  border-radius: 99px;
  background: #252A36;
  overflow: hidden;
}
#s2-bar-fill {
  height: 100%;
  background: #FF4F1A;
  border-radius: 99px;
  width: 0%;
}
#s2-exe-card {
  background: #132620;
  border: 1.5px solid #10B981;
  border-radius: 18px;
  padding: 20px 24px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

/* ==========================================================================
   SCENE 03A: NO MANUAL FREES / NO DANGLING POINTERS
   ========================================================================== */
.s3a-wrap {
  position: absolute;
  left: -490px;
  top: -230px;
  width: 980px;
  height: 460px;
  color: #0B0B10;
}
.s3a-header {
  display: flex;
  justify-content: space-between;
  align-items: baseline;
  margin-bottom: 24px;
}
.s3a-title {
  font-size: 38px;
  font-weight: 800;
  letter-spacing: -0.03em;
}
.hz-row {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 22px 28px;
  margin-bottom: 16px;
  border-radius: 18px;
  background: #F6F5F0;
  border: 1.5px solid #E2E0D8;
}
.hz-code {
  font-size: 24px;
  font-weight: 700;
  color: #111318;
}
.hz-strike {
  position: absolute;
  left: 24px;
  top: 50%;
  height: 4px;
  background: #FF4F1A;
  border-radius: 2px;
  width: 0%;
}
.hz-badge {
  padding: 8px 16px;
  border-radius: 99px;
  font-size: 14px;
  font-weight: 700;
  letter-spacing: 0.08em;
}
.hz-badge.warn { background: #FEE2E2; color: #DC2626; }
.hz-badge.done { background: #DCFCE7; color: #15803D; }

/* ==========================================================================
   SCENE 03B: 24-BYTE ARC HEADER + PLACEMENT + CYCLE COLLECTOR
   ========================================================================== */
.s3b-wrap {
  position: absolute;
  left: -610px;
  top: -295px;
  width: 1220px;
  height: 590px;
  color: #F4F2EE;
  display: flex;
  flex-direction: column;
  gap: 22px;
}
.arc-header-box {
  background: #181B22;
  border: 1px solid #2B3040;
  border-radius: 18px;
  padding: 20px 24px;
}
.arc-cells {
  display: grid;
  grid-template-columns: 1fr 1fr 1fr 1.5fr;
  gap: 12px;
  margin-top: 12px;
}
.arc-cell {
  background: #111318;
  border: 1.5px solid #2E3446;
  border-radius: 12px;
  padding: 14px 16px;
}
.arc-cell.payload {
  border-color: #FF4F1A;
  background: rgba(255, 79, 26, 0.08);
}
.s3b-bottom {
  display: flex;
  gap: 22px;
  flex: 1;
}
.s3b-tiers {
  flex: 0.95;
  background: #181B22;
  border: 1px solid #2B3040;
  border-radius: 18px;
  padding: 20px 24px;
  position: relative;
}
.tier-list {
  position: relative;
  margin-top: 14px;
  height: 210px;
}
#s3b-tier-pill {
  position: absolute;
  left: 0;
  right: 0;
  background: rgba(255, 79, 26, 0.16);
  border: 1.5px solid #FF4F1A;
  border-radius: 14px;
}
.tier-item {
  position: relative;
  height: 60px;
  margin-bottom: 12px;
  padding: 0 18px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  z-index: 2;
}
.s3b-cycles {
  flex: 1.15;
  background: #181B22;
  border: 1px solid #2B3040;
  border-radius: 18px;
  padding: 20px 24px;
  display: flex;
  flex-direction: column;
  justify-content: space-between;
}
.cycle-nodes {
  display: flex;
  align-items: center;
  justify-content: space-around;
  margin: 18px 0;
}
.cyc-node {
  width: 165px;
  padding: 16px 14px;
  border-radius: 16px;
  background: #111318;
  border: 2px solid #FF4F1A;
  text-align: center;
}

/* ==========================================================================
   SCENE 04: PIPELINE & 3 BACKENDS
   ========================================================================== */
.s4-wrap {
  position: absolute;
  left: -630px;
  top: -290px;
  width: 1260px;
  height: 580px;
  color: #F4F2EE;
  display: flex;
  flex-direction: column;
  justify-content: space-between;
}
.s4-stations {
  position: relative;
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 18px;
  padding-top: 26px;
}
.s4-st {
  background: #12151E;
  border: 2px solid #262A36;
  border-radius: 18px;
  padding: 20px 18px;
}
#s4-hero-orb {
  position: absolute;
  left: 50%;
  top: 50%;
  width: 26px;
  height: 26px;
  margin-left: -13px;
  border-radius: 50%;
  background: #FFFFFF;
  box-shadow: 0 0 0 6px rgba(255, 79, 26, 0.35);
  z-index: 5;
}
.s4-backends {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.s4-lane {
  background: #151922;
  border: 1px solid #262B38;
  border-radius: 16px;
  padding: 16px 22px;
  display: flex;
  align-items: center;
  gap: 22px;
}
.s4-lane-bar {
  flex: 1;
  height: 12px;
  background: #0B0D12;
  border-radius: 99px;
  overflow: hidden;
}
.s4-lane-fill {
  height: 100%;
  border-radius: 99px;
  width: 0%;
}

/* ==========================================================================
   SCENE 05: DIRECT LINKER & KEEL WORKSPACE MANAGER
   ========================================================================== */
.s5-wrap {
  position: absolute;
  left: -610px;
  top: -275px;
  width: 1220px;
  height: 550px;
  display: flex;
  gap: 26px;
  color: #0B0B10;
}
.s5-col {
  flex: 1;
  background: #F6F5F0;
  border: 1.5px solid #E2E0D8;
  border-radius: 20px;
  padding: 24px 26px;
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.s5-cmd-box {
  background: #14171F;
  border: 1px solid #262B38;
  border-radius: 12px;
  padding: 13px 18px;
  color: #E5E7EB;
  font-size: 14px;
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.fmt-card {
  background: #111318;
  color: #F4F2EE;
  border-radius: 14px;
  padding: 18px 22px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-top: 12px;
}
.pkg-row {
  background: #FFFFFF;
  border: 1px solid #E2E0D8;
  border-radius: 14px;
  padding: 16px 20px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-top: 12px;
}

/* ==========================================================================
   SCENE 06: SAFETY S2 & E4403 REJECTION GATE
   ========================================================================== */
.s6-wrap {
  position: absolute;
  left: -610px;
  top: -285px;
  width: 1220px;
  height: 570px;
  display: flex;
  gap: 26px;
  color: #F4F2EE;
}
.s6-panel {
  flex: 1;
  background: #181B22;
  border: 1px solid #2A2F3D;
  border-radius: 20px;
  padding: 24px 26px;
  display: flex;
  flex-direction: column;
  justify-content: space-between;
}
.proof-row {
  background: #111318;
  border: 1.5px solid #262A36;
  border-radius: 14px;
  padding: 16px 18px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-top: 12px;
}
.gate-stage {
  position: relative;
  height: 220px;
  background: #111318;
  border: 1px solid #262A36;
  border-radius: 16px;
  display: flex;
  align-items: center;
  justify-content: center;
  overflow: hidden;
}
#s6-unsafe-pkt {
  padding: 14px 20px;
  border-radius: 12px;
  background: #3B1219;
  border: 2px solid #EF4444;
  color: #FCA5A5;
  font-weight: 700;
  font-size: 16px;
}
#s6-shield-bar {
  width: 10px;
  height: 160px;
  border-radius: 99px;
  background: #FF4F1A;
  margin: 0 24px;
}
.exe-slot {
  padding: 14px 20px;
  border-radius: 12px;
  border: 1.5px dashed #525866;
  color: #71717A;
  font-size: 15px;
}

/* ==========================================================================
   SCENE 07: KINETIC TRIPLET & END CARD
   ========================================================================== */
#scene7-overlay {
  position: absolute;
  inset: 0;
  z-index: 8;
}
#s7-triplet {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  overflow: hidden;
}
#s7-triplet-ghost {
  position: absolute;
  font-size: 290px;
  font-weight: 900;
  letter-spacing: -0.05em;
  white-space: nowrap;
  pointer-events: none;
}
#s7-triplet-word {
  font-size: 196px;
  font-weight: 900;
  letter-spacing: -0.05em;
  line-height: 0.95;
  z-index: 2;
}
#s7-triplet-sub {
  margin-top: 24px;
  font-size: 22px;
  letter-spacing: 0.2em;
  font-weight: 700;
  z-index: 2;
}
#s7-endcard {
  position: absolute;
  inset: 0;
  background: #0B0B10;
  color: #EDEBE4;
  display: flex;
  align-items: center;
  justify-content: center;
}
#s7-end-inner {
  text-align: center;
}
.end-logo {
  font-size: 180px;
  font-weight: 900;
  letter-spacing: -0.055em;
  line-height: 1;
}
.end-logo span { color: #FF4F1A; }
.end-tagline {
  font-size: 42px;
  font-weight: 600;
  margin-top: 16px;
  color: #EDEBE4;
}
.end-rule {
  width: 520px;
  height: 1px;
  background: rgba(237, 235, 228, 0.2);
  margin: 28px auto;
}
#s7-end-cli {
  font-size: 20px;
  color: #FF4F1A;
  letter-spacing: 0.08em;
  min-height: 28px;
}

/* ==========================================================================
   CURSOR & CAMERA VIEWFINDER HUD
   ========================================================================== */
#cursor {
  position: absolute;
  left: 0;
  top: 0;
  width: 44px;
  height: auto;
  transform-origin: 3px 3px;
  filter: drop-shadow(0 3px 5px rgba(0,0,0,0.28));
  z-index: 15;
  pointer-events: none;
}
#hud-layer {
  position: absolute;
  inset: 28px 36px;
  pointer-events: none;
  z-index: 20;
  font-family: 'Geist Mono', monospace;
  font-size: 14px;
  letter-spacing: 0.14em;
}
.hud-br {
  position: absolute;
  width: 18px;
  height: 18px;
  border-color: currentColor;
  opacity: 0.55;
}
.hud-br.tl { left: 0; top: 0; border-left: 2px solid; border-top: 2px solid; }
.hud-br.tr { right: 0; top: 0; border-right: 2px solid; border-top: 2px solid; }
.hud-br.bl { left: 0; bottom: 0; border-left: 2px solid; border-bottom: 2px solid; }
.hud-br.br { right: 0; bottom: 0; border-right: 2px solid; border-bottom: 2px solid; }
.hud-tl { position: absolute; left: 26px; top: 4px; font-weight: 700; }
.hud-tr { position: absolute; right: 26px; top: 4px; font-weight: 700; }
.hud-bl { position: absolute; left: 26px; bottom: 4px; font-weight: 700; }
.hud-br-text {
  position: absolute;
  right: 26px;
  bottom: 4px;
  display: flex;
  align-items: center;
  gap: 10px;
  font-weight: 700;
}
.rec-dot {
  width: 9px;
  height: 9px;
  border-radius: 50%;
  background: #FF4F1A;
}
#hud-caption-pill {
  position: absolute;
  left: 50%;
  bottom: 12px;
  transform: translateX(-50%);
  background: rgba(11, 11, 16, 0.88);
  color: #EDEBE4;
  padding: 12px 28px;
  border-radius: 999px;
  border: 1px solid rgba(255,255,255,0.12);
  font-family: 'Geist', sans-serif;
  font-size: 21px;
  font-weight: 500;
  letter-spacing: -0.01em;
  white-space: nowrap;
  max-width: 1480px;
  overflow: hidden;
  text-overflow: ellipsis;
}
.cap-w { opacity: 0.52; }
.cap-w.spoken { opacity: 0.92; }
.cap-w.active {
  opacity: 1;
  color: #FF4F1A;
  font-weight: 700;
}

/* Preview Control Bar (hidden in ?render) */
#preview-controls {
  position: fixed;
  left: 0;
  right: 0;
  bottom: 0;
  height: 48px;
  background: #0B0B10;
  border-top: 1px solid #262934;
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 0 24px;
  color: #EDEBE4;
  font-family: 'Geist Mono', monospace;
  font-size: 13px;
  z-index: 100;
}
body.render #preview-controls { display: none !important; }
#ctrl-play {
  background: #FF4F1A;
  color: #FFFFFF;
  border: none;
  border-radius: 6px;
  padding: 6px 16px;
  font-family: 'Geist', sans-serif;
  font-weight: 700;
  cursor: pointer;
}
#ctrl-scrub { flex: 1; accent-color: #FF4F1A; cursor: pointer; }
</style>
</head>
<body>
<div id="wrap">
  <div id="stage">
    <div id="bg-grid"></div>
    <div id="bg-vignette"></div>

    <!-- SCENE 01 OVERLAY -->
    <div id="scene1-overlay">
      <div id="s1-spark-wrap">
        <div id="s1-spark-label" class="mono">L++ // REWRITE COMPILER — 2026</div>
        <div id="s1-spark-bar"></div>
      </div>
      <div id="s1-wait-block">
        <div class="wait-pill mono">
          <span data-icon="clock" data-size="22" data-color="#FF4F1A"></span>
          <span id="s1-wait-timer">compiling... 2.4s</span>
          <div id="s1-wait-strike"></div>
        </div>
        <div class="s1-headline">Code shouldn't wait<br/>for the compiler.</div>
      </div>
      <div id="s1-brand-block">
        <div class="s1-kicker mono">INTRODUCING — LPLUSPLUS v1.0</div>
        <div id="s1-lpp-title">L<span class="plus">++</span></div>
        <div class="s1-sub">Clean indented syntax · Compile-time ARC · Millisecond native builds</div>
      </div>
    </div>

    <!-- CONTINUOUS ONE-SHAPE WORLD (SCENES 02 - 06) -->
    <div id="world">
      <div id="shape">

        <!-- LAYER 2: SYNTAX & MILLISECOND BUILD -->
        <div class="L" id="L-scene2">
          <div class="s2-wrap">
            <div class="s2-editor">
              <div class="s2-tabbar">
                <div class="s2-dots">
                  <i style="background:#EF4444"></i>
                  <i style="background:#F59E0B"></i>
                  <i style="background:#10B981"></i>
                </div>
                <div class="mono" style="font-size:14px;color:#9CA3AF">src/main.lpp — UTF-8 · Spaces Only</div>
                <div class="mono" style="font-size:13px;color:#FF4F1A;font-weight:700">:= INFERRED</div>
              </div>
              <div id="s2-code-lines"></div>
            </div>

            <div class="s2-right">
              <div id="s2-build-btn">
                <span style="display:flex;align-items:center;gap:10px">
                  <span data-icon="terminal" data-size="22" data-color="#FFFFFF"></span>
                  <span id="s2-build-label" class="mono">lpp main.lpp --run</span>
                </span>
                <span data-icon="arrow" data-size="20" data-color="#FFFFFF"></span>
              </div>

              <div class="s2-telemetry">
                <div class="s2-ms-label mono">CRANELIFT AOT + DIRECT LINK</div>
                <div id="s2-ms-num" class="mono">3.4 ms</div>
                <div class="s2-bar-track"><div id="s2-bar-fill"></div></div>
                <div class="mono" style="margin-top:10px;font-size:13px;color:#9CA3AF">2–5 ms standalone native executable</div>
              </div>

              <div id="s2-exe-card">
                <div style="display:flex;align-items:center;justify-content:space-between">
                  <span class="mono" style="font-size:18px;font-weight:700;color:#10B981">main.exe · READY</span>
                  <span data-icon="check" data-size="22" data-color="#10B981"></span>
                </div>
                <div class="mono" style="font-size:15px;color:#D1FAE5">stdout &gt; Distance: 5.000000 (exit 0)</div>
              </div>
            </div>
          </div>
        </div>

        <!-- LAYER 3A: MEMORY CONTRACT -->
        <div class="L" id="L-scene3a">
          <div class="s3a-wrap">
            <div class="s3a-header">
              <div class="s3a-title">Memory handled at compile time.</div>
              <div class="mono" style="font-size:15px;color:#FF4F1A;font-weight:700">ZERO MANUAL FREE()</div>
            </div>
            <div class="hz-row">
              <div class="hz-code mono">free(raw_ptr) // manual deallocation</div>
              <div class="hz-strike" id="s3a-strike-0"></div>
              <div class="hz-badge warn mono" id="s3a-badge-0">MANUAL HAZARD</div>
            </div>
            <div class="hz-row">
              <div class="hz-code mono">ptr-&gt;field // dangling pointer dereference</div>
              <div class="hz-strike" id="s3a-strike-1"></div>
              <div class="hz-badge warn mono" id="s3a-badge-1">MANUAL HAZARD</div>
            </div>
            <div class="hz-row">
              <div class="hz-code mono">use_after_move(val) // double release</div>
              <div class="hz-strike" id="s3a-strike-2"></div>
              <div class="hz-badge warn mono" id="s3a-badge-2">MANUAL HAZARD</div>
            </div>
          </div>
        </div>

        <!-- LAYER 3B: 24-BYTE ARC HEADER + PLACEMENT + CYCLE COLLECTOR -->
        <div class="L" id="L-scene3b">
          <div class="s3b-wrap">
            <div class="arc-header-box">
              <div style="display:flex;justify-content:space-between;align-items:center">
                <span style="font-size:20px;font-weight:800">24-Byte Invariant ARC Header Layout</span>
                <span class="mono" style="font-size:14px;color:#FF4F1A">ptr points to +24 payload</span>
              </div>
              <div class="arc-cells mono">
                <div class="arc-cell">
                  <div style="font-size:12px;color:#9CA3AF">OFFSET +0 (8B)</div>
                  <div style="font-size:18px;font-weight:700;margin-top:4px;color:#60A5FA">rc: i64 (Atomic)</div>
                </div>
                <div class="arc-cell">
                  <div style="font-size:12px;color:#9CA3AF">OFFSET +8 (8B)</div>
                  <div style="font-size:18px;font-weight:700;margin-top:4px;color:#FBBF24">drop_fn: ptr</div>
                </div>
                <div class="arc-cell">
                  <div style="font-size:12px;color:#9CA3AF">OFFSET +16 (8B)</div>
                  <div style="font-size:18px;font-weight:700;margin-top:4px;color:#34D399">magic: "ARC1"</div>
                </div>
                <div class="arc-cell payload">
                  <div style="font-size:12px;color:#FF4F1A">OFFSET +24 (OBJECT PAYLOAD)</div>
                  <div style="font-size:18px;font-weight:700;margin-top:4px;color:#FFFFFF">Struct Fields / UTF-8 Bytes</div>
                </div>
              </div>
            </div>

            <div class="s3b-bottom">
              <div class="s3b-tiers">
                <div style="font-size:18px;font-weight:800">3-Tier Ownership Placement</div>
                <div class="tier-list">
                  <div id="s3b-tier-pill"></div>
                  <div class="tier-item">
                    <span style="font-size:18px;font-weight:700">1 · Frame (Stack)</span>
                    <span class="mono" style="font-size:14px;color:#34D399">Non-escaping · 0 RC</span>
                  </div>
                  <div class="tier-item">
                    <span style="font-size:18px;font-weight:700">2 · Owned (Heap)</span>
                    <span class="mono" style="font-size:14px;color:#60A5FA">Move retain elision</span>
                  </div>
                  <div class="tier-item">
                    <span style="font-size:18px;font-weight:700">3 · Shared (SCC)</span>
                    <span class="mono" style="font-size:14px;color:#FF4F1A">Atomic RC + Collector</span>
                  </div>
                </div>
              </div>

              <div class="s3b-cycles">
                <div style="display:flex;justify-content:space-between;align-items:center">
                  <span style="font-size:18px;font-weight:800">Bacon–Rajan Trial Deletion</span>
                  <span id="s3b-cycle-status" class="mono" style="font-size:14px;font-weight:700;color:#FF4F1A">1 · ACTIVE CYCLE</span>
                </div>
                <div class="cycle-nodes mono">
                  <div class="cyc-node" id="s3b-node-0">
                    <div style="font-size:16px;font-weight:700">Node A</div>
                    <div id="s3b-rc-0" style="font-size:14px;color:#9CA3AF;margin-top:6px">rc: 2</div>
                  </div>
                  <div style="color:#FF4F1A;font-weight:700">⇄</div>
                  <div class="cyc-node" id="s3b-node-1">
                    <div style="font-size:16px;font-weight:700">Node B</div>
                    <div id="s3b-rc-1" style="font-size:14px;color:#9CA3AF;margin-top:6px">rc: 2</div>
                  </div>
                  <div style="color:#FF4F1A;font-weight:700">⇄</div>
                  <div class="cyc-node" id="s3b-node-2">
                    <div style="font-size:16px;font-weight:700">Node C</div>
                    <div id="s3b-rc-2" style="font-size:14px;color:#9CA3AF;margin-top:6px">rc: 2</div>
                  </div>
                </div>
                <div class="mono" style="font-size:13px;color:#9CA3AF;text-align:center">
                  lpp_trace_* + lpp_arc_collect_cycles() reclaims unreachable Shared cycles
                </div>
              </div>
            </div>
          </div>
        </div>

        <!-- LAYER 4: PIPELINE & 3 BACKENDS -->
        <div class="L" id="L-scene4">
          <div class="s4-wrap">
            <div style="display:flex;justify-content:space-between;align-items:baseline">
              <div style="font-size:32px;font-weight:800">Unidirectional 15-Crate Pipeline</div>
              <div class="mono" style="font-size:15px;color:#FF4F1A">REWRITE ENGINE · MIR-v7</div>
            </div>

            <div class="s4-stations">
              <div id="s4-hero-orb"></div>
              <div class="s4-st" id="s4-st-0">
                <div class="mono" style="font-size:13px;color:#FF4F1A">01 · FRONTEND</div>
                <div style="font-size:22px;font-weight:800;margin:6px 0">Parse &amp; HIR</div>
                <div class="mono" style="font-size:13px;color:#9CA3AF">lpp-frontend · lpp-hir</div>
              </div>
              <div class="s4-st" id="s4-st-1">
                <div class="mono" style="font-size:13px;color:#60A5FA">02 · SEMANTICS</div>
                <div style="font-size:22px;font-weight:800;margin:6px 0">Type Check</div>
                <div class="mono" style="font-size:13px;color:#9CA3AF">lpp-types · Monomorph</div>
              </div>
              <div class="s4-st" id="s4-st-2">
                <div class="mono" style="font-size:13px;color:#FBBF24">03 · SSA CFG</div>
                <div style="font-size:22px;font-weight:800;margin:6px 0">Build MIR</div>
                <div class="mono" style="font-size:13px;color:#9CA3AF">lpp-mir · lpp-passes</div>
              </div>
              <div class="s4-st" id="s4-st-3">
                <div class="mono" style="font-size:13px;color:#34D399">04 · VERIFIER</div>
                <div style="font-size:22px;font-weight:800;margin:6px 0">Prove Ownership</div>
                <div class="mono" style="font-size:13px;color:#9CA3AF">lpp-ownership · E4402</div>
              </div>
            </div>

            <div class="s4-backends">
              <div class="s4-lane" id="s4-lane-card-0">
                <div style="width:240px;font-size:20px;font-weight:800;color:#FF4F1A">Cranelift Backend</div>
                <div class="s4-lane-bar"><div class="s4-lane-fill" id="s4-lane-fill-0" style="background:#FF4F1A"></div></div>
                <div class="mono" style="width:360px;font-size:14px;color:#D1D5DB">x86_64 · aarch64 · Instant Native + Cycle GC</div>
              </div>
              <div class="s4-lane" id="s4-lane-card-1">
                <div style="width:240px;font-size:20px;font-weight:800;color:#60A5FA">LLVM Backend</div>
                <div class="s4-lane-bar"><div class="s4-lane-fill" id="s4-lane-fill-1" style="background:#60A5FA"></div></div>
                <div class="mono" style="width:360px;font-size:14px;color:#D1D5DB">--backend llvm · Optimizing Scalar/CFG Subset</div>
              </div>
              <div class="s4-lane" id="s4-lane-card-2">
                <div style="width:240px;font-size:20px;font-weight:800;color:#34D399">WebAssembly</div>
                <div class="s4-lane-bar"><div class="s4-lane-fill" id="s4-lane-fill-2" style="background:#34D399"></div></div>
                <div class="mono" style="width:360px;font-size:14px;color:#D1D5DB">wasm32-wasip1 · Size-Class ARC Free-List Reuse</div>
              </div>
            </div>
          </div>
        </div>

        <!-- LAYER 5: DIRECT LINKER & KEEL WORKSPACE MANAGER -->
        <div class="L" id="L-scene5">
          <div class="s5-wrap">
            <div class="s5-col">
              <div>
                <div class="mono" style="font-size:14px;color:#FF4F1A;font-weight:700">ZERO-DEPENDENCY LINKER</div>
                <div style="font-size:30px;font-weight:800;margin-top:4px">lpp-link Direct Binary</div>
              </div>
              <div class="s5-cmd-box mono">
                <span><span style="color:#FF4F1A">$</span> lpp main.lpp -o app.exe</span>
                <span style="color:#10B981;font-weight:700">NO HOST LD / MSVC</span>
              </div>
              <div>
                <div class="fmt-card" id="s5-fmt-0">
                  <span class="mono" style="font-size:20px;font-weight:800;color:#FF4F1A">ELF64</span>
                  <span class="mono" style="font-size:15px;color:#9CA3AF">Linux Native Executable</span>
                </div>
                <div class="fmt-card" id="s5-fmt-1">
                  <span class="mono" style="font-size:20px;font-weight:800;color:#60A5FA">PE32+</span>
                  <span class="mono" style="font-size:15px;color:#9CA3AF">Windows Native .exe</span>
                </div>
                <div class="fmt-card" id="s5-fmt-2">
                  <span class="mono" style="font-size:20px;font-weight:800;color:#34D399">Mach-O</span>
                  <span class="mono" style="font-size:15px;color:#9CA3AF">macOS arm64 / x86_64</span>
                </div>
              </div>
            </div>

            <div class="s5-col">
              <div style="display:flex;justify-content:space-between;align-items:center">
                <div>
                  <div class="mono" style="font-size:14px;color:#2F47FF;font-weight:700">PACKAGE &amp; WORKSPACE MANAGER</div>
                  <div style="font-size:30px;font-weight:800;margin-top:4px">Keel · Keel.toml</div>
                </div>
                <div class="mono" style="background:#0B0B10;color:#FFF;padding:10px 16px;border-radius:10px;font-size:14px;font-weight:700">
                  keel build
                </div>
              </div>
              <div class="s5-cmd-box mono">
                <span><span style="color:#60A5FA">$</span> keel build --workspace --locked</span>
                <span style="color:#60A5FA;font-weight:700">DETERMINISTIC DAG</span>
              </div>
              <div>
                <div class="pkg-row" id="s5-pkg-0">
                  <span class="mono" style="font-size:17px;font-weight:700">lpp-bindgen · v0.36.0</span>
                  <span id="s5-pkg-chk-0" class="mono" style="color:#15803D;font-weight:700">SHA-256 LOCKED ✓</span>
                </div>
                <div class="pkg-row" id="s5-pkg-1">
                  <span class="mono" style="font-size:17px;font-weight:700">lpp-semver · v0.1.0</span>
                  <span id="s5-pkg-chk-1" class="mono" style="color:#15803D;font-weight:700">SHA-256 LOCKED ✓</span>
                </div>
                <div class="pkg-row" id="s5-pkg-2">
                  <span class="mono" style="font-size:17px;font-weight:700">workspace DAG · Keel.lock</span>
                  <span id="s5-pkg-chk-2" class="mono" style="color:#15803D;font-weight:700">VERIFIED ✓</span>
                </div>
              </div>
            </div>
          </div>
        </div>

        <!-- LAYER 6: SAFETY MISSION S2 & REJECTION GATE -->
        <div class="L" id="L-scene6">
          <div class="s6-wrap">
            <div class="s6-panel">
              <div>
                <div class="mono" style="font-size:13px;color:#10B981;font-weight:700">SAFETY MISSION LEVEL S2</div>
                <div style="font-size:28px;font-weight:800;margin-top:4px">Independent Proof Check</div>
              </div>
              <div>
                <div class="proof-row" id="s6-proof-0">
                  <span class="mono" style="font-size:16px">E4401 · Ownership Plan Built</span>
                  <span class="mono" id="s6-pbadge-0" style="padding:6px 12px;border-radius:99px;font-size:13px;font-weight:700">VERIFIED ✓</span>
                </div>
                <div class="proof-row" id="s6-proof-1">
                  <span class="mono" style="font-size:16px">E4402 · Independent Recompute</span>
                  <span class="mono" id="s6-pbadge-1" style="padding:6px 12px;border-radius:99px;font-size:13px;font-weight:700">MATCHES ✓</span>
                </div>
                <div class="proof-row" id="s6-proof-2">
                  <span class="mono" style="font-size:16px">537 / 537 Workspace Tests</span>
                  <span class="mono" id="s6-pbadge-2" style="padding:6px 12px;border-radius:99px;font-size:13px;font-weight:700">191/191 CORPUS ✓</span>
                </div>
              </div>
            </div>

            <div class="s6-panel">
              <div>
                <div class="mono" style="font-size:13px;color:#FF4F1A;font-weight:700">S1 / S2 REJECTION CONTRACT</div>
                <div style="font-size:28px;font-weight:800;margin-top:4px">Unprovable Code Rejected</div>
              </div>
              <div class="gate-stage mono">
                <div id="s6-unsafe-pkt">use_after_move(x)</div>
                <div id="s6-shield-bar"></div>
                <div class="exe-slot">output.exe<br/>(blocked)</div>
              </div>
              <div id="s6-reject-banner" class="mono" style="background:rgba(255,79,26,0.16);border:1.5px solid #FF4F1A;color:#FF4F1A;padding:14px 16px;border-radius:12px;font-size:13px;font-weight:700;text-align:center;white-space:nowrap">
                REJECTED · E4403 Ownership Balance Failure — 0 Bytes Emitted
              </div>
            </div>
          </div>
        </div>

      </div>
    </div>

    <!-- CURSOR -->
    <svg id="cursor" viewBox="0 0 28 28" fill="none" xmlns="http://www.w3.org/2000/svg">
      <path d="M3 2.5L22.5 12.2L13.2 14.8L9.8 24.2L3 2.5Z" fill="#0B0B10" stroke="#FFFFFF" stroke-width="2.2" stroke-linejoin="round"/>
    </svg>

    <!-- SCENE 07 OVERLAY (KINETIC TRIPLET -> END CARD) -->
    <div id="scene7-overlay" style="display:none">
      <div id="s7-triplet">
        <div id="s7-triplet-ghost">L++   L++</div>
        <div id="s7-triplet-word">L++</div>
        <div id="s7-triplet-sub" class="mono">SYSTEMS COMPILER · v1.0</div>
      </div>
      <div id="s7-endcard" style="display:none">
        <div id="s7-end-inner">
          <div class="end-logo">L<span>++</span></div>
          <div class="end-tagline">Write it. Prove it. Ship it.</div>
          <div class="end-rule"></div>
          <div id="s7-end-cli" class="mono"></div>
        </div>
      </div>
    </div>

    <!-- CAMERA VIEWFINDER HUD -->
    <div id="hud-layer">
      <div class="hud-br tl"></div>
      <div class="hud-br tr"></div>
      <div class="hud-br bl"></div>
      <div class="hud-br br"></div>
      <div class="hud-tl">L++ // MOTION SPEC · 1920×1080</div>
      <div class="hud-tr" id="hud-scene-idx">SCENE 01 / 07</div>
      <div class="hud-bl" id="hud-category">SPARK // BRAND SLAM</div>
      <div id="hud-caption-pill"></div>
      <div class="hud-br-text">
        <span class="rec-dot"></span>
        <span id="hud-timecode">00:00:00</span>
      </div>
    </div>
  </div>
</div>

<!-- PREVIEW CONTROLS (Hidden in ?render) -->
<div id="preview-controls">
  <button id="ctrl-play">Play</button>
  <input id="ctrl-scrub" type="range" min="0" max="84.5" step="0.03" value="0" />
  <span id="ctrl-time">0.0s / 84.5s</span>
  <audio id="master-audio" src="audio/master_mix.wav" preload="auto"></audio>
</div>

<script>${bundleJs}</script>
</body>
</html>`;

fs.writeFileSync(path.join(ROOT, 'lpp-trailer.html'), html, 'utf8');
console.log('Built lpp-trailer.html (' + html.length + ' bytes)');
