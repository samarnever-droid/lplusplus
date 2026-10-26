// render.mjs — Barty-Bart Motion-Graphics Renderer (Stills Contact Sheet + Multi-Worker MP4 + Audio Mux)
import { spawn } from 'node:child_process';
import { writeFileSync, mkdirSync, rmSync, existsSync } from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { chromium } = require('C:/Users/khati/node_modules/playwright');

const [, , html, out, a3 = '30', a4 = '1', a5 = '6'] = process.argv;
if (!html || !out) {
  console.log('usage: node render.mjs page.html out.mp4 [fps=30] [subframes=1] [workers=6]');
  console.log('       node render.mjs page.html sheet.png --stills 1.0,5.0,12.0,...');
  process.exit(1);
}

const run = (args) =>
  new Promise((res, rej) => {
    const p = spawn('ffmpeg', args, { stdio: 'inherit' });
    p.on('close', (c) => (c ? rej(new Error('ffmpeg exited with ' + c)) : res()));
  });

const url = pathToFileURL(path.resolve(html)).href + '?render';
const candidates = [
  'C:/Users/khati/AppData/Local/ms-playwright/chromium_headless_shell-1228/chrome-headless-shell-win64/chrome-headless-shell.exe',
  'C:/Users/khati/AppData/Local/ms-playwright/chromium-1228/chrome-win64/chrome.exe',
  'C:/Program Files/Google/Chrome/Application/chrome.exe',
];
const localChromium = candidates.find((p) => existsSync(p));
const browser = await chromium.launch({
  ...(localChromium ? { executablePath: localChromium } : {}),
  args: ['--allow-file-access-from-files'],
});

const probe = await browser.newPage();
const pageErrors = [];
probe.on('pageerror', (e) => pageErrors.push(e.message));
await probe.goto(url);
await probe.waitForFunction(() => window.VIDEO && typeof window.seek === 'function');
const V = await probe.evaluate(() => window.VIDEO);
await probe.close();

if (pageErrors.length) {
  console.error('Page errors during probe:', pageErrors);
  process.exit(1);
}

async function openPage() {
  const page = await browser.newPage({
    viewport: { width: V.w, height: V.h },
    deviceScaleFactor: 1,
  });
  page.on('pageerror', (e) => pageErrors.push(e.message));
  await page.goto(url);
  await page.waitForFunction(() => window.VIDEO && typeof window.seek === 'function');
  const bad = await page.evaluate(async () => {
    await Promise.all([...document.fonts].map((f) => f.load().catch(() => {})));
    await document.fonts.ready;
    return [...document.fonts].filter((f) => f.status !== 'loaded').map((f) => f.family);
  });
  if (bad.length) {
    console.warn('WARNING: fonts did not load:', [...new Set(bad)].join(', '));
  }
  return page;
}

const shot = async (page, t) => {
  await page.evaluate((timeVal) => window.seek(timeVal), t);
  return page.screenshot({ type: 'jpeg', quality: 94 });
};

if (a3 === '--stills') {
  const page = await openPage();
  const times = a4.split(',').map(Number);
  const dir = out + '.frames';
  mkdirSync(dir, { recursive: true });
  for (let i = 0; i < times.length; i++) {
    writeFileSync(`${dir}/${String(i).padStart(3, '0')}.jpg`, await shot(page, times[i]));
  }
  await browser.close();
  const cols = Math.min(3, times.length);
  const rows = Math.ceil(times.length / cols);
  await run([
    '-v',
    'error',
    '-y',
    '-framerate',
    '1',
    '-i',
    `${dir}/%03d.jpg`,
    '-vf',
    `scale=640:-1,tile=${cols}x${rows}:padding=6:color=#222226`,
    '-frames:v',
    '1',
    out,
  ]);
  rmSync(dir, { recursive: true, force: true });
  console.log('stills ->', out);
  process.exit(0);
}

const fps = +a3;
const sub = Math.max(1, +a4);
const workers = Math.max(1, +a5);
const N = Math.round(V.duration * fps);
const base = out.replace(/\.mp4$/, '');
const silent = base + '_silent.mp4';
const vf =
  sub > 1
    ? ['-vf', `tmix=frames=${sub},select='eq(mod(n\\,${sub})\\,${sub - 1})',setpts=N/(${fps}*TB)`]
    : [];

const t0 = Date.now();
let doneFrames = 0;
const per = Math.ceil(N / workers);
const segs = [];

await Promise.all(
  Array.from({ length: workers }, async (_, w) => {
    const a = w * per;
    const b = Math.min(N, a + per);
    if (a >= b) return;
    const segFile = `${base}_seg${w}.mp4`;
    segs[w] = segFile;
    const page = await openPage();
    const ff = spawn(
      'ffmpeg',
      [
        '-v',
        'error',
        '-y',
        '-f',
        'image2pipe',
        '-framerate',
        String(fps * sub),
        '-i',
        '-',
        ...vf,
        '-r',
        String(fps),
        '-c:v',
        'libx264',
        '-pix_fmt',
        'yuv420p',
        '-crf',
        '16',
        '-preset',
        'fast',
        segFile,
      ],
      { stdio: ['pipe', 'inherit', 'inherit'] }
    );
    const closed = new Promise((r) => ff.on('close', r));
    for (let f = a; f < b; f++) {
      for (let s = 0; s < sub; s++) {
        const buf = await shot(page, (f + (sub > 1 ? (s / sub) * 0.5 : 0)) / fps);
        if (!ff.stdin.write(buf)) await new Promise((r) => ff.stdin.once('drain', r));
      }
      if (++doneFrames % 60 === 0) {
        process.stdout.write(`\rRendered ${doneFrames}/${N} frames (${((Date.now() - t0) / 1000).toFixed(1)}s)`);
      }
    }
    ff.stdin.end();
    await closed;
    await page.close();
  })
);

await browser.close();

const list = base + '_segs.txt';
writeFileSync(
  list,
  segs
    .filter(Boolean)
    .map((s) => `file '${path.resolve(s).replace(/\\/g, '/')}'`)
    .join('\n')
);
await run(['-v', 'error', '-y', '-f', 'concat', '-safe', '0', '-i', list, '-c', 'copy', silent]);
segs.filter(Boolean).forEach((s) => rmSync(s, { force: true }));
rmSync(list, { force: true });
console.log(`\nSilent video -> ${silent} (${((Date.now() - t0) / 1000).toFixed(1)}s)`);

// Mux with audio/master_mix.wav (Sarvam TTS + Google Flow Music sidechain ducked)
const masterWav = path.resolve(path.dirname(html), 'audio/master_mix.wav');
if (existsSync(masterWav)) {
  await run([
    '-v',
    'error',
    '-y',
    '-i',
    silent,
    '-i',
    masterWav,
    '-af',
    'loudnorm=I=-16:TP=-1.5:LRA=11',
    '-c:v',
    'copy',
    '-c:a',
    'aac',
    '-b:a',
    '192k',
    '-movflags',
    '+faststart',
    '-shortest',
    out,
  ]);
  console.log('Final video with audio ->', out);
}
