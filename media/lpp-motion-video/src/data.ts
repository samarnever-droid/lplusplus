/*
 * L++ 2D MOTION-GRAPHICS TRAILER — BEAT SHEET (120 BPM, 84.50s)
 * =========================================================================================================
 * Time (s)     | Cue   | Spoken Line / Event                              | Visual Shape / Transition / Sound
 * -------------|-------|--------------------------------------------------|----------------------------------
 * 0.00 -  1.80 | INTRO | [Spark ignites on dark #0B0B10]                  | Spark pulses -> horizontal rule
 * 0.80 -  7.12 | vo_01 | "Every second spent waiting on a compiler..."    | Rule floods -> STOP WAITING -> L++ Slam
 * 7.45 -  8.12 | XFER1 | [+ glyph detaches -> morphs into editor card]    | One-shape morph into IDE window
 * 8.12 - 16.73 | vo_02 | "A compiled systems language with clean..."      | L++ code types at 28ch/s + camera push
 * 17.23 - 22.10| vo_03 | "Write it, build it, and get a real native..."   | Cursor clicks Build -> 3.4 ms -> main.exe
 * 23.10 - 28.56| vo_04 | "Memory is handled for you. No manual frees..."  | Shape -> white card; free() crossed out
 * 29.06 - 39.30| vo_05 | "Reference counting is planned at compile..."    | 24B ARC header + Frame/Owned/Shared + Cycle
 * 39.90 - 47.84| vo_06 | "Under the hood is one verified pipeline..."     | Hero orb rides Parse -> HIR -> MIR -> Proof
 * 48.34 - 55.93| vo_07 | "Then Cranelift for instant native builds..."    | Splits into Cranelift / LLVM / WASM lanes
 * 56.43 - 64.19| vo_08 | "Its own linker writes E L F, P E, and Mach-O.." | lpp-link stamps ELF/PE/Mach-O + Keel lock
 * 65.19 - 70.74| vo_09 | "And it's honest about safety. Every ownership.."| Independent E4401 + E4402 proof checks
 * 71.24 - 76.62| vo_10 | "Anything the compiler can't prove, it rejects.."| Unsafe graph slams into E4403 Shield (0 B)
 * 77.62 - 81.29| vo_11 | "L plus plus. Write it. Prove it. Ship it."      | Beat-locked Kinetic Triplet -> Dot Bridge
 * 81.29 - 84.50| OUTRO | [Reverb tail + End Card hold]                    | L++ End Card + command line typing
 * =========================================================================================================
 */

export interface VoCue {
  id: string;
  start: number;
  end: number;
  text: string;
  scene: number;
}

export const VIDEO_DURATION = 84.5;

export const VO_CUES: VoCue[] = [
  {
    id: 'vo_01',
    start: 0.80,
    end: 7.12,
    text: 'Every second spent waiting on a compiler is a second not spent building. Meet L++.',
    scene: 1,
  },
  {
    id: 'vo_02',
    start: 8.12,
    end: 16.73,
    text: 'A compiled systems language with clean, indented syntax that reads like a script, and runs like native code.',
    scene: 2,
  },
  {
    id: 'vo_03',
    start: 17.23,
    end: 22.10,
    text: 'Write it, build it, and get a real native executable in milliseconds.',
    scene: 2,
  },
  {
    id: 'vo_04',
    start: 23.10,
    end: 28.56,
    text: 'Memory is handled for you. No manual frees. No dangling pointers to chase.',
    scene: 3,
  },
  {
    id: 'vo_05',
    start: 29.06,
    end: 39.30,
    text: 'Reference counting is planned at compile time. Values that never escape stay on the stack, and a cycle collector reclaims anything that loops back on itself.',
    scene: 3,
  },
  {
    id: 'vo_06',
    start: 39.90,
    end: 47.84,
    text: 'Under the hood is one verified pipeline. Parse, type check, build MIR, and prove ownership.',
    scene: 4,
  },
  {
    id: 'vo_07',
    start: 48.34,
    end: 55.93,
    text: 'Then Cranelift for instant native builds, with LLVM and WebAssembly backends alongside.',
    scene: 4,
  },
  {
    id: 'vo_08',
    start: 56.43,
    end: 64.19,
    text: 'Its own linker writes ELF, PE, and Mach-O directly. And Keel manages your packages and workspaces.',
    scene: 5,
  },
  {
    id: 'vo_09',
    start: 65.19,
    end: 70.74,
    text: "And it's honest about safety. Every ownership proof is checked independently.",
    scene: 6,
  },
  {
    id: 'vo_10',
    start: 71.24,
    end: 76.62,
    text: "Anything the compiler can't prove, it rejects, before a binary ever exists.",
    scene: 6,
  },
  {
    id: 'vo_11',
    start: 77.62,
    end: 81.29,
    text: 'L++. Write it. Prove it. Ship it.',
    scene: 7,
  },
];

export interface SceneMeta {
  index: number;
  tag: string;
  category: string;
  start: number;
  end: number;
}

export const SCENES: SceneMeta[] = [
  { index: 1, tag: 'SCENE 01 / 07', category: 'SPARK // BRAND SLAM', start: 0.0, end: 7.75 },
  { index: 2, tag: 'SCENE 02 / 07', category: 'SYNTAX // MILLISECOND BUILD', start: 7.75, end: 22.65 },
  { index: 3, tag: 'SCENE 03 / 07', category: 'MEMORY // COMPILE-TIME ARC', start: 22.65, end: 39.60 },
  { index: 4, tag: 'SCENE 04 / 07', category: 'PIPELINE // MULTI-BACKEND', start: 39.60, end: 56.10 },
  { index: 5, tag: 'SCENE 05 / 07', category: 'DIRECT LINKER // KEEL PM', start: 56.10, end: 64.75 },
  { index: 6, tag: 'SCENE 06 / 07', category: 'SAFETY S2 // PROOF VERIFIER', start: 64.75, end: 77.15 },
  { index: 7, tag: 'SCENE 07 / 07', category: 'KINETIC TRIPLET // END CARD', start: 77.15, end: 84.50 },
];

export const LPP_CODE_LINES: Array<{ indent: number; tokens: Array<[string, string]> }> = [
  { indent: 0, tokens: [['# main.lpp — Indented, statically typed L++', 'cm']] },
  { indent: 0, tokens: [['struct ', 'kw'], ['Point', 'ty'], [':', 'op']] },
  { indent: 1, tokens: [['x', 'id'], [': ', 'op'], ['Float', 'ty']] },
  { indent: 1, tokens: [['y', 'id'], [': ', 'op'], ['Float', 'ty'], [' = ', 'op'], ['0.0', 'nu']] },
  { indent: 0, tokens: [['impl ', 'kw'], ['Point', 'ty'], [':', 'op']] },
  { indent: 1, tokens: [['def ', 'kw'], ['magnitude', 'fn'], ['(self) -> ', 'op'], ['Float', 'ty'], [':', 'op']] },
  { indent: 2, tokens: [['return ', 'kw'], ['sqrt', 'fn'], ['(self.x * self.x + self.y * self.y)', 'id']] },
  { indent: 0, tokens: [['def ', 'kw'], ['main', 'fn'], ['():', 'op']] },
  { indent: 1, tokens: [['p ', 'id'], [':= ', 'kw'], ['Point', 'ty'], ['(', 'op'], ['3.0', 'nu'], [', ', 'op'], ['4.0', 'nu'], [')', 'op']] },
  { indent: 1, tokens: [['dist ', 'id'], [':= ', 'kw'], ['p.', 'id'], ['magnitude', 'fn'], ['()', 'op']] },
  { indent: 1, tokens: [['print_str', 'fn'], ['(', 'op'], ['f"Distance: {dist}"', 'st'], [')', 'op']] },
];
