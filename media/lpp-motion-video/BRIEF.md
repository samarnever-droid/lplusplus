# L++ — 80-second 2D motion-graphics trailer

Status: waiting for audio. Drop the two generated files into `audio/`:

- `audio/music.mp3` (or .wav) — from the MUSIC PROMPT below
- `audio/voiceover.mp3` (or .wav) — from the TTS SCRIPT below

Once both exist, the animation is built to this timing map (final timing follows the real voiceover file).

## Timing map (120 BPM: 1 beat = 0.5 s, 1 bar = 2 s, 40 bars total)

| Time | Music section | Voiceover | On-screen motion |
|---|---|---|---|
| 0:00–0:08 | Intro (4 bars) | Line 1 | Dark grid, cursor blink, `L++` logo assembles from code glyphs |
| 0:08–0:24 | Build A (8 bars) | Lines 2–3 | Indented code types itself, build bar fills, "ms" timer snaps, .exe icon pops |
| 0:24–0:32 | Build B + riser (4 bars) | Line 4 | Memory blocks float in, "free()" crossed out |
| 0:32–0:56 | DROP (12 bars) | Lines 5–8 | Stack vs heap split, ARC header, cycle loop dissolves; pipeline stages light up; three backend lanes; ELF / PE / Mach-O cards fly out; Keel boxes stack |
| 0:56–1:08 | Breakdown (6 bars) | Lines 9–10 | Calm: proof checkmarks tick; red unsafe shape bounces off a shield before the .exe appears |
| 1:08–1:20 | Final hit + outro (6 bars) | Line 11 | "Write it. Prove it. Ship it." on beats, logo lockup, fade to black |

## MUSIC PROMPT

```
Instrumental only, no vocals, no lyrics, no spoken words, no vocal chops.
Genre: modern tech/synthwave hybrid for a 2D motion-graphics product trailer.
Tempo: exactly 120 BPM, 4/4 time, key of A minor. Total length: exactly 80 seconds.
Mix: clean, punchy, leave midrange space for a voiceover (no lead melody louder than the voice).

Structure (strict timing):
0:00–0:08 Intro: dark analog synth pad, soft ticking closed hi-hats, subtle digital glitch textures, slow filter opening.
0:08–0:24 Build A: pulsing sidechained sub bass on eighth notes, bright arpeggiated plucks enter, light percussion.
0:24–0:32 Build B: add claps on 2 and 4, rising white-noise riser and snare roll during the last 2 seconds.
0:32–0:56 Drop: full energy, four-on-the-floor kick, wide supersaw chords, driving bass, crisp percussion; add a small fill every 8 bars (at 0:40 and 0:48).
0:56–1:08 Breakdown: drums drop out, warm pad and gentle pluck motif only, calm and confident, riser in the final 2 seconds.
1:08–1:20 Final section: one big impact hit exactly at 1:08, energetic 4-bar resolution, then a clean ending with a reverb tail fading to silence at 1:20.

Avoid: vocals, humming, choirs, lo-fi hiss, abrupt cut-off, tempo changes.
```

## TTS VOICE PROMPT (style/voice settings)

```
Confident, warm, modern narrator, clear American English, cinematic product-trailer tone.
Medium pace (about 150 words per minute), crisp consonants, slight energy lift on short punchy phrases,
natural pauses at periods, no breathiness, no music, no sound effects, dry studio recording.
Pronounce "L plus plus" as "ell plus plus", "Cranelift" as "CRANE-lift", "Mach-O" as "mack-oh", "MIR" as "M I R".
```

## TTS SCRIPT (paste exactly; remove the [pause] lines if your TTS doesn't support tags)

```
Every second spent waiting on a compiler is a second not spent building. Meet L plus plus.
[pause 1s]
A compiled systems language with clean, indented syntax that reads like a script, and runs like native code.
[pause 0.5s]
Write it, build it, and get a real native executable in milliseconds.
[pause 1s]
Memory is handled for you. No manual frees. No dangling pointers to chase.
[pause 0.5s]
Reference counting is planned at compile time. Values that never escape stay on the stack, and a cycle collector reclaims anything that loops back on itself.
[pause 0.5s]
Under the hood is one verified pipeline. Parse, type check, build M I R, and prove ownership.
[pause 0.5s]
Then Cranelift for instant native builds, with LLVM and WebAssembly backends alongside.
[pause 0.5s]
Its own linker writes E L F, P E, and Mach-O directly. And Keel manages your packages and workspaces.
[pause 1s]
And it's honest about safety. Every ownership proof is checked independently.
[pause 0.5s]
Anything the compiler can't prove, it rejects, before a binary ever exists.
[pause 1s]
L plus plus. Write it. Prove it. Ship it.
```

Target length: about 65–72 seconds of speech including pauses (~165 words).
If your TTS output runs past 1:15, raise speed slightly rather than cutting lines.

## Claim discipline

Wording stays within documented evidence (`documentation/Safety_Mission.md` rule 10):
no "as safe as Rust", no "data-race free", no "no GC" (there is a cycle collector),
LLVM/WASM described only as backends, not as feature parity with Cranelift.
