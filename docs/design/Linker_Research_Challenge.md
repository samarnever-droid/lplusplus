# Linker Research Challenge — Replace 99% of `lpp-link` at Its Speed

> **Audience:** This document is self-contained. You do not need prior knowledge
> of L++ or lpp-link. Read sections 1–2 before starting.

## 1. Background — what you are trying to beat

**L++** is a native AOT programming language (compiler written in Rust, backend:
Cranelift). Its compiler emits standard object files — **COFF** on Windows,
**ELF** on Linux, **Mach-O** on macOS — plus a small precompiled C runtime
object (`lpp_runtime_min.obj` / `.o`, ~110 KB).

**`lpp-link`** is L++'s in-house direct linker: a single 530 KB static binary
that turns `program.obj + lpp_runtime_min.obj` into a runnable executable
**without invoking any host C toolchain** (no MSVC `link.exe`, no `gcc`, no
`vcvars64.bat`). It writes PE32+, ELF, and Mach-O outputs directly, including:

- PE: `.text/.rdata/.data/.tls/.idata/.reloc` sections, import descriptors for
  KERNEL32 / USER32 / GDI32 / msvcrt / WS2_32 resolved **by symbol name alone**,
  base relocations, TLS directory, synthesized PE entry reaching `lpp_main`
- ELF: GOT/rodata/data merging, dynamic entry with a `_start` stub calling the
  program entry
- Mach-O writer (macOS)

**Why it is fast.** Three design facts, not magic:

1. **Zero external processes.** It is the *only* process in the link step.
2. **Minimal feature surface.** It implements exactly the subset L++ needs —
   no linker scripts, no LTO, no archive resolution, no garbage collection of
   sections beyond what the workload requires.
3. **It sits behind a cache layer.** L++'s build system caches compiled objects
   by content hash (`Cache hit: <hash>`), and the runtime object is compiled
   once into `~/.lpp/lib/`. On an unchanged-source rebuild, **compilation is
   skipped entirely and only the link runs.** Any replacement must preserve the
   user-visible result of this pipeline, measured in Section 3 Tier B.

## 2. The exact interface a replacement must honor

Invocation today (Windows shown):

```
lpp-link pe <program.obj> <path-to/lpp_runtime_min.obj> -o <output.exe>
```

- Inputs: one COFF object from the compiler + one COFF runtime object.
  (Same shape for `elf` / `macho` modes on Linux/macOS.)
- Objects reference runtime symbols like `lpp_arc_alloc`, `lpp_print_str`,
  and OS APIs by bare name (`GetStdHandle`, `printf`, `WSAStartup`, ...).
  The linker must resolve runtime symbols from the runtime object and satisfy
  OS API symbols by **synthesizing import table entries from the name alone**
  (there are no `.lib` import libraries available — the zero-dependency rule).
- Output must start and run correctly on a machine with no dev tools installed.

## 3. Measured baseline (the bar)

Machine: 12th Gen Intel Core i3-1215U (6-core low-power laptop), Windows,
warm filesystem cache, `lpp-link` built with `cargo build --release`
(530,944 bytes). Median of 21 runs per workload.
Reproducible: `challenge_bench/bench_lpp_link.ps1`,
`challenge_bench/bench_cached_build.ps1` in the L++ repository.

### Tier A — raw link time (`lpp-link pe obj runtime -o out`)

| Workload | Object size | Median | Min | Max |
|---|---|---|---|---|
| hello_world | 6.6 KB | **27.6 ms** | 18.7 ms | 35.2 ms |
| fibonacci | 6.6 KB | **26.5 ms** | 13.4 ms | 27.9 ms |
| network_echo_client (needs WS2_32 imports) | 7.6 KB | **27.4 ms** | 15.6 ms | 30.3 ms |
| big_gen (3000 functions) | 108 KB | **28.2 ms** | 16.5 ms | 36.5 ms |

Note the flatness: 6 KB → 108 KB objects all link in ~27 ms. The floor is
dominated by Windows process launch (~15–20 ms), with the linker's own work in
the low single-digit milliseconds. A candidate that is a long-lived/daemon
process may beat the wall time, but must still be benchmarked wall-clock,
invocation-to-file-on-disk, the same way.

### Tier B — cached end-to-end rebuild (`lpp build`, cache hit + link)

| Metric | Value |
|---|---|
| Median of 21 runs | **49.6 ms** |
| Min / Max | 42.2 ms / 90.0 ms |

(Tier B uses a debug build of the driver binary; a release driver would be
faster. The point is the *order of magnitude* the user experiences.)

## 4. Hard requirements (fail any one = disqualified)

| # | Requirement | Verification |
|---|---|---|
| R1 | Tri-platform output from one toolchain: Linux x86-64 ELF, Windows x86-64 PE32+, macOS Mach-O (arm64 + x86-64) | Link hello_world on each OS; run it |
| R2 | Consumes Cranelift-emitted COFF/ELF/Mach-O objects + the 110 KB runtime object **unmodified** | No preprocessing/conversion allowed |
| R3 | Zero host toolchain: no `link.exe`, no `vcvars64.bat`, no gcc/clang driver, no `ld` shell-outs | Clean Windows VM with only the candidate binary |
| R4 | PE imports synthesized from bare symbol names (KERNEL32, USER32, GDI32, msvcrt, WS2_32) without `.lib` files | `network_echo_client` links & runs |
| R5 | ELF dynamic output: GOT/PLT, interpreter path, merged rodata/data | fibonacci + struct/list workload run correctly |
| R6 | Entry/ABI correctness: PE entry reaching the runtime's `lpp_main`-style entry, ELF `_start`, Mach-O `LC_MAIN` | Program starts on real hardware |
| R7 | Permissive license (MIT / Apache-2.0 / BSD) — redistributable bundled inside L++ | Read the LICENSE |
| R8 | Single static binary ≤ 50 MB per platform, no installer, no runtime DLL deps | Directory listing; `otool -L` / dependency check |
| R9 | TLS support (PE TLS directory or equivalent) | A TLS-using program links and runs |

## 5. Speed gate

Same machine class or better; warm cache; median of 21 runs; wall clock,
invocation-to-file-on-disk.

- **Tier A pass:** candidate median ≤ **1.25 × lpp-link median** (i.e. ≤ ~35 ms
  on the hello_world class) on **both PE and ELF**.
- **Tier B pass:** cached rebuild with candidate ≤ **1.25 × 49.6 ms**.
- **Large tier (report, not required):** link a ≥10 MB-object workload; if the
  candidate is ≥2× faster there, record it — that changes the recommendation
  even with a slight small-link loss.
- Cold start (fresh boot / cleared file cache) must also be recorded.

## 6. Correctness gate — behavioral parity

For a representative suite (all runnable `tests/*.lpp`, the 20-workload king20
benchmark set, and `safety/s0` + `safety/s1` rejection suites):

```
link with candidate → run → capture stdout + exit code
link with lpp-link  → run → capture stdout + exit code
PASS only if identical on 100% of cases
```

Also spot-check the binaries: PE import table contains the expected DLLs;
ELF `.dynamic` section sane; output runs on a machine with no dev tools.

## 7. The allowed 1% (only these may differ)

- The built-in `inspect` diagnostic mode (nice-to-have, not required)
- Custom ELF `_start` stub injection — the runtime may provide `_start`
  instead if the candidate needs it
- Response-file / CLI syntax specifics

Everything else — R1–R9, speed gates, correctness parity — is non-negotiable.

## 8. Known candidates and their current blockers (start here)

| Candidate | Status vs. requirements | Action |
|---|---|---|
| **lld** (LLVM) | Only candidate covering R1+R4+R7 today. Risk: Tier A small-link overhead (full-featured startup cost). Shares Zig's blockers below when fed our objects unmodified | Benchmark it first — the favorite |
| **mold** | Fails R1/R4: no Windows PE output yet (tracked in rui314/mold issue #190); also GPL-licensed since v2 (fails R7) | Re-check every release |
| **Wild** (Rust) | Fails R1: Linux-only; Mach-O/Windows explicitly unimplemented. MIT/Apache; best codebase to study | Watch wild-linker/wild |
| **Zig self-hosted linker** (tested 0.16.0, 2026-08-09) | **Fails R2 as a drop-in** — see experiment log below | Requires compiler-side changes first |
| **Go cmd/link** (tested go1.26.4) | **Disqualified by design**: accepts only Go-compiler objects (Go package metadata required); foreign-object linking goes through `-extld` → host linker, failing R3 | Skip |
| **MSVC link.exe / Apple ld-prime** | Fail R1/R3/R7: single-platform, proprietary | Skip |

### 8.1 Zig extraction experiment — 2026-08-09 (zig 0.16.0, Windows)

Tested `zig build-exe hello_world.obj lpp_runtime_min.obj -target
x86_64-windows-gnu -fentry=main` against our Tier A objects. Findings:

1. **Zig's PE path is embedded LLD**, not the self-hosted linker (self-hosted
   covers ELF/Mach-O; Windows COFF goes through lld-link compiled into zig.exe).
2. **Our objects declare ~20 runtime externs that are never called**
   (lpp_net_*, lpp_gui_*, lpp_json_*, ...). lpp-link tolerates this; LLD
   rejects it. LLD's escape hatch `/force:unresolved` exists, but Zig's CLI
   whitelist refuses to pass it through (`error: unsupported linker arg`).
   Workaround proved viable only with preprocessing: a hand-written `stubs.obj`
   providing the unused symbols links cleanly.
3. **Cranelift COFF objects embed MSVC `/DEFAULTLIB:LIBCMT` directives**; with
   `-target x86_64-windows-gnu` lld-link demands `libLIBCMT.a` and fails.
   The MSVC flavor of the target instead requires a host MSVC install (R3 fail).
4. **Entry is `main`, not `lpp_main`**: the program object defines a C-style
   `main`; lpp-link *synthesizes* the PE entry around it. Zig's `-fentry=main`
   handles this part fine.
5. Size: zig.exe distribution ≈ 100 MB → fails R8 (≤ 50 MB) unless a custom
   stripped build is produced.

Conclusion: Zig (and therefore raw lld) cannot consume L++ objects
**unmodified**. Adoption requires compiler-side adaptations — all small:
stop emitting unused extern declarations, drop `/DEFAULTLIB` directives
(or link against the MSVC target with bundled import libs), keep `main` as
entry. These adaptations are equally valid for plain lld and are the cheapest
path to any external linker.

Repro artifacts in the repo: `challenge_bench/` (objects, `stubs.c`, bench
scripts, `zigdist/` not committed).

## 9. Deliverable

A one-page scorecard:

```
Candidate:            _______
R1–R9:                9/9  |  which failed: _______
Tier A median (PE):   ___ ms   (lpp-link: 27.6 ms)
Tier A median (ELF):  ___ ms   (lpp-link: TBD on Linux box)
Tier B median:        ___ ms   (lpp-link pipeline: 49.6 ms)
Large-tier link:      ___ ms vs lpp-link ___ ms
King20 parity:        __ / 20
License:              _______
Binary size:          ___ MB
Cold-start median:    ___ ms
```

Decision rule:

- **9/9 + both speed gates + 20/20 parity** → adopt as the new default
  (`lpp build --linker <name>`), keep lpp-link as the zero-dependency fallback.
- **Otherwise** → lpp-link stays; the scorecard becomes the roadmap for what
  to steal (mold's parallelism, Wild's incremental design, lld's breadth).

## 10. lpp-link capability audit — what it does NOT support, per OS (2026-08-10)

Audited line-by-line from `src/bin/lpp-link.rs` (2,335 lines). A replacement
must at minimum match the ✅ rows; the ❌ rows are the real reason this
challenge exists.

### 10.1 Feature × OS matrix

| Feature | Windows (PE) | Linux (ELF) | macOS (Mach-O) |
|---|---|---|---|
| Input parsing | ✅ COFF x86-64 + `.lib`/`.a` archives | ✅ ELF relocatable x86-64 | ⚠️ Mach-O x86-64, **`__text` only** |
| Data sections (.data/.rdata/__DATA) | ✅ merged per class | ✅ merged | ❌ **dropped — code-only programs link** |
| BSS / zero-fill | ✅ | ✅ | ❌ |
| TLS | ✅ `.tls` + `IMAGE_TLS_DIRECTORY` | ❌ | ❌ |
| Relocations | ✅ 11 AMD64 types (ADDR64/32/32NB, REL32_0-5, SECTION, SECREL) | ✅ x86-64 abs/PC-rel | ⚠️ 4-byte PC-relative only, ±2 GB |
| Dynamic linking / shared libs | ❌ import-table only | ❌ static, no PT_INTERP | ❌ no dyld, no dylib |
| OS API imports | ✅ name-based synthesis into KERNEL32/USER32/GDI32/WS2_32/msvcrt (hardcoded whitelist) | ❌ none — raw syscalls only | ❌ none |
| Entry point | ✅ synthesized PE entry | ✅ `_start` | ✅ requires `main`, LC_MAIN |
| Base relocations / ASLR tables | ✅ `.reloc` DIR64 | n/a (static) | n/a |
| SEH / unwinding (.pdata/.xdata) | ❌ **explicitly discarded** (no stack unwinding) | n/a | ❌ |
| Resources / manifest / icons (.rsrc) | ❌ | n/a | n/a |
| Debug info (DWARF/CodeView/PDB) | ❌ sections skipped | ❌ | ❌ |
| Weak symbols / COMDAT dedup | ❌ duplicate definition = hard error | ❌ | ❌ |
| DLL / .so / dylib output | ❌ executables only | ❌ | ❌ |
| Exports | ❌ | ❌ | ❌ |
| Incremental link / map file / gc-sections | ❌ | ❌ | ❌ |
| Architectures | x86-64 only | x86-64 only | x86-64 only |
| Response files (@args.rsp) | ✅ all modes | ✅ | ✅ |
| Format autodetect (no subcommand) | ✅ magic sniffing | ✅ | ✅ |

### 10.2 Feature vs stability (per backend)

| Backend | Maturity | Stability | Evidence |
|---|---|---|---|
| PE (Windows) | ~1,250 lines, deepest | **High** — exercised on every `lpp build`; king20 20/20, safety s0+s1 pass | Daily driver; tolerant of unused externs; archives + TLS + base relocs all real |
| ELF (Linux) | ~420 lines | **Medium** — works for raw-syscall static binaries; never the daily driver | Single RWX PT_LOAD segment, no libc; fine for L++ runtime model but untested breadth |
| Mach-O (macOS) | ~240 lines | **Low / untested** — text-only merge, sets `MH_DYLDLINK\|MH_TWOLEVEL` flags with no dylib commands (risky), never run on real macOS hardware | Any program with data literals beyond what fits in code streams will fail |

### 10.3 Gap ranking (what a replacement buys us)

1. **Mach-O data sections + dyld/libSystem imports** — current path cannot link real programs.
2. **ELF dynamic output or at least libc-free breadth** (threads/TLS, futex-based runtime).
3. **ARM64 (Apple Silicon + Windows ARM + Linux aarch64)** — zero support anywhere.
4. **SEH/unwinding tables on PE** — no structured exception handling today.
5. **Weak/COMDAT, debug info passthrough, resources** — standard linker features, all absent.
6. Subsystem control, exports/DLLs, incremental linking — nice-to-have tier.

A candidate that passes R1–R9 inherits gaps 1–6 for free; that is the actual
value proposition of this challenge.
