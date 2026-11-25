import { useState } from "react";
import { motion } from "framer-motion";
import {
  ArrowRight,
  Check,
  Copy,
  Cpu,
  Lock,
  Package,
  Play,
  Sparkles,
  Terminal,
} from "lucide-react";
import { EASE } from "../lib/ui";

const CODE_TABS = [
  {
    id: "capabilities",
    label: "capabilities.lpp",
    badge: "@pure & @deny",
    code: `# Zero-runtime-cost capability security
@pure
def hash_slice(s: Str, k: Int) -> Int:
    head := s[0..5]
    return (len(head) * 8) + (k ** 2)

@deny(fs, net, exec, ffi, unsafe)
def render_tag(name: Str, active: Bool) -> Str:
    status := "live" if active else "idle"
    return f"{name[..6]}:{status}"

def main():
    print(hash_slice("lplusplus", 5))
    print_str(render_tag("kernel_v3", true))`,
    output: [
      "$ lpp capabilities.lpp --run",
      "65",
      "kernel:live",
      "✓ Verified @pure & @deny transitively · 0.9ms native build",
    ],
  },
  {
    id: "keel",
    label: "keel.sh",
    badge: "Keel + Registry",
    code: `# Create, verify & run with Keel package manager
keel new my-service && cd my-service

# Search & install from the canonical git registry
keel search semver
keel add lpp-semver
keel install && keel verify

# Incremental multi-backend build (Cranelift / LLVM / WASM)
keel build && keel run`,
    output: [
      "resolved 1 package(s) → Keel.lock",
      "lpp-semver 1.0.0  ed937ef36169  ok",
      "build OK (1 package(s))",
      "Hello from L++",
    ],
  },
  {
    id: "kernel",
    label: "kernel.lpp",
    badge: "Bare-Metal OS",
    code: `# Principled unsafe & x86_64-unknown-none ELF kernels
@export_name("_start")
@section(".text.boot")
unsafe def kmain() -> Void:
    vga := (0xB8000 as USize) as Ptr[U16]
    ptr_write[U16](vga, (0x0F4C as U16))
    port_out8(0x3F8 as U16, 0x4C as U8)
    cpu_hlt()`,
    output: [
      "$ lpp kernel.lpp --target x86_64-unknown-none --multiboot2 -o kernel.elf",
      "✓ Linked static freestanding ELF64 (lpp-link)",
      "✓ 0 runtime dependencies · 0 GC pauses",
    ],
  },
];

const INSTALL_TABS = [
  {
    id: "unix",
    label: "macOS / Linux",
    cmd: "curl -fsSL https://lplusplus.bond/install.sh | sh",
  },
  {
    id: "win",
    label: "Windows",
    cmd: "irm https://lplusplus.bond/install.ps1 | iex",
  },
  {
    id: "source",
    label: "Source",
    cmd: "git clone --depth 1 https://github.com/samarnever-droid/lplusplus.git && cd lplusplus && LPP_FROM_SOURCE=1 sh install.sh",
  },
];

const METRICS = [
  { label: "Compile Backends", value: "Cranelift · LLVM · WASM" },
  { label: "Corpus Pass Rate", value: "193 / 193 (100%)" },
  { label: "Workspace Tests", value: "894 Passing" },
  { label: "GC Pauses", value: "0 ms (Deterministic)" },
];

export default function Hero() {
  const [codeTab, setCodeTab] = useState(0);
  const [installTab, setInstallTab] = useState(0);
  const [copied, setCopied] = useState(false);

  const activeCode = CODE_TABS[codeTab];
  const activeInstall = INSTALL_TABS[installTab];

  const copyInstall = async () => {
    try {
      await navigator.clipboard.writeText(activeInstall.cmd);
    } catch {
      /* ignore */
    }
    setCopied(true);
    setTimeout(() => setCopied(false), 1800);
  };

  return (
    <section id="top" className="relative overflow-hidden pt-28 pb-20 md:pt-36 md:pb-28">
      {/* Clerk-inspired dot grid + electric indigo top aurora */}
      <div className="pointer-events-none absolute inset-0 bg-grid opacity-55 [mask-image:radial-gradient(ellipse_at_top,black_30%,transparent_75%)]" />
      <div className="pointer-events-none absolute -top-48 left-1/2 h-[520px] w-[960px] -translate-x-1/2 rounded-full bg-[#6c47ff]/20 blur-[140px]" />
      <div className="pointer-events-none absolute top-40 right-0 h-[360px] w-[360px] rounded-full bg-[#38bdf8]/10 blur-[130px]" />

      <div className="relative mx-auto max-w-7xl px-5 md:px-8">
        <div className="grid items-center gap-12 lg:grid-cols-12">
          {/* Left Column */}
          <div className="space-y-7 lg:col-span-6">
            <motion.a
              href="#install"
              initial={{ opacity: 0, y: 12 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ duration: 0.55, ease: EASE }}
              className="group inline-flex items-center gap-2 rounded-full border border-[#6c47ff]/35 bg-[#6c47ff]/10 px-3.5 py-1.5 text-xs font-medium text-zinc-200 transition-colors hover:border-[#6c47ff]/60 hover:bg-[#6c47ff]/15"
            >
              <span className="h-2 w-2 rounded-full bg-[#8b6bff] animate-pulse-dot" />
              <span className="font-mono text-[#a78bfa]">v0.3.2</span>
              <span className="text-zinc-500">·</span>
              <span>@pure/@deny Security, Algebraic MIR &amp; Keel</span>
              <ArrowRight className="h-3.5 w-3.5 text-[#a78bfa] transition-transform group-hover:translate-x-0.5" />
            </motion.a>

            <motion.h1
              initial={{ opacity: 0, y: 16 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ duration: 0.65, delay: 0.08, ease: EASE }}
              className="font-display text-4xl font-semibold leading-[1.04] tracking-[-0.035em] text-white sm:text-5xl lg:text-[3.75rem]"
            >
              The systems language built for{" "}
              <span className="bg-gradient-to-r from-[#a78bfa] via-[#7c5cff] to-[#38bdf8] bg-clip-text text-transparent">
                deterministic speed
              </span>
              .
            </motion.h1>

            <motion.p
              initial={{ opacity: 0, y: 16 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ duration: 0.65, delay: 0.14, ease: EASE }}
              className="max-w-xl text-base leading-relaxed text-zinc-400 sm:text-lg"
            >
              Python-clean syntax compiled straight to native binaries via{" "}
              <span className="text-zinc-200">Cranelift</span>,{" "}
              <span className="text-zinc-200">LLVM</span>, and{" "}
              <span className="text-zinc-200">WebAssembly</span>. Zero GC pauses, compile-time{" "}
              <code className="rounded bg-white/[0.06] px-1.5 py-0.5 font-mono text-xs text-[#a78bfa]">
                @pure
              </code>{" "}
              capability sandboxing, bare-metal OS primitives, and the{" "}
              <span className="text-zinc-200">Keel</span> package manager.
            </motion.p>

            {/* Primary CTAs */}
            <motion.div
              initial={{ opacity: 0, y: 14 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ duration: 0.65, delay: 0.2, ease: EASE }}
              className="flex flex-wrap items-center gap-3"
            >
              <a
                href="#install"
                className="clerk-btn-primary inline-flex items-center gap-2 rounded-xl px-5 py-3 text-sm font-medium transition-all"
              >
                <Terminal className="h-4 w-4" />
                Install L++ &amp; Keel
                <ArrowRight className="h-4 w-4" />
              </a>
              <a
                href={`${import.meta.env.BASE_URL}account.html`}
                className="inline-flex items-center gap-2 rounded-xl border border-white/10 bg-white/[0.04] px-5 py-3 text-sm font-medium text-zinc-200 transition-colors hover:border-white/20 hover:bg-white/[0.08] hover:text-white"
              >
                <Sparkles className="h-4 w-4 text-[#a78bfa]" />
                Publisher Studio
              </a>
              <a
                href={`${import.meta.env.BASE_URL}packages.html`}
                className="inline-flex items-center gap-2 rounded-xl border border-white/10 bg-transparent px-4 py-3 text-sm font-medium text-zinc-400 transition-colors hover:border-white/15 hover:text-white"
              >
                <Package className="h-4 w-4" />
                Packages
              </a>
            </motion.div>

            {/* Clerk-style compact installer widget */}
            <motion.div
              initial={{ opacity: 0, y: 14 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ duration: 0.65, delay: 0.26, ease: EASE }}
              className="clerk-card overflow-hidden rounded-2xl"
            >
              <div className="flex items-center justify-between border-b border-white/[0.07] px-3 py-2">
                <div className="flex items-center gap-1">
                  {INSTALL_TABS.map((t, idx) => (
                    <button
                      key={t.id}
                      onClick={() => setInstallTab(idx)}
                      className={`rounded-md px-2.5 py-1 text-xs font-medium transition-colors ${
                        installTab === idx
                          ? "bg-[#6c47ff]/20 text-white"
                          : "text-zinc-400 hover:text-zinc-200"
                      }`}
                    >
                      {t.label}
                    </button>
                  ))}
                </div>
                <button
                  onClick={copyInstall}
                  className="inline-flex items-center gap-1.5 rounded-md border border-white/10 bg-white/[0.04] px-2.5 py-1 font-mono text-[11px] text-zinc-300 transition-colors hover:border-[#6c47ff]/50 hover:text-white"
                >
                  {copied ? (
                    <Check className="h-3 w-3 text-emerald-400" />
                  ) : (
                    <Copy className="h-3 w-3" />
                  )}
                  {copied ? "Copied" : "Copy"}
                </button>
              </div>
              <div className="overflow-x-auto bg-[#09090b]/80 px-4 py-3 font-mono text-xs text-zinc-200">
                <span className="mr-2 select-none text-[#8b6bff]">$</span>
                {activeInstall.cmd}
              </div>
            </motion.div>
          </div>

          {/* Right Column: Interactive Clerk-Style IDE Card */}
          <motion.div
            initial={{ opacity: 0, y: 20 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.75, delay: 0.16, ease: EASE }}
            className="lg:col-span-6"
          >
            <div className="clerk-card overflow-hidden rounded-2xl">
              {/* IDE Window Header */}
              <div className="flex flex-wrap items-center justify-between gap-2 border-b border-white/[0.08] bg-white/[0.02] px-4 py-3">
                <div className="flex items-center gap-1.5">
                  <span className="h-2.5 w-2.5 rounded-full bg-zinc-700" />
                  <span className="h-2.5 w-2.5 rounded-full bg-zinc-700" />
                  <span className="h-2.5 w-2.5 rounded-full bg-zinc-700" />
                </div>
                <div className="flex items-center gap-1">
                  {CODE_TABS.map((tab, idx) => (
                    <button
                      key={tab.id}
                      onClick={() => setCodeTab(idx)}
                      className={`rounded-lg px-2.5 py-1 font-mono text-[11px] transition-all ${
                        codeTab === idx
                          ? "border border-[#6c47ff]/40 bg-[#6c47ff]/15 text-white"
                          : "border border-transparent text-zinc-400 hover:text-zinc-200"
                      }`}
                    >
                      {tab.label}
                    </button>
                  ))}
                </div>
                <span className="hidden rounded-full border border-white/10 bg-white/[0.04] px-2.5 py-0.5 font-mono text-[10px] text-[#a78bfa] sm:inline-block">
                  {activeCode.badge}
                </span>
              </div>

              {/* Code View */}
              <pre className="code-scroll overflow-x-auto bg-[#09090b]/90 p-5 font-mono text-[12.5px] leading-6 text-zinc-200">
                <code>{activeCode.code}</code>
              </pre>

              {/* Terminal Output Footer */}
              <div className="border-t border-white/[0.08] bg-[#111116] px-5 py-4">
                <div className="mb-2 flex items-center justify-between">
                  <span className="inline-flex items-center gap-1.5 font-mono text-[11px] uppercase tracking-wider text-zinc-400">
                    <Play className="h-3 w-3 text-[#8b6bff]" />
                    Output
                  </span>
                  <span className="inline-flex items-center gap-1 font-mono text-[11px] text-emerald-400">
                    <Lock className="h-3 w-3" />
                    Deterministic
                  </span>
                </div>
                <div className="space-y-1 font-mono text-xs">
                  {activeCode.output.map((line, i) => (
                    <div
                      key={i}
                      className={
                        line.startsWith("$")
                          ? "text-zinc-400"
                          : line.startsWith("✓")
                            ? "text-emerald-400"
                            : "text-zinc-200"
                      }
                    >
                      {line}
                    </div>
                  ))}
                </div>
              </div>
            </div>

            {/* Trust Strip */}
            <div className="mt-4 grid grid-cols-2 gap-3 sm:grid-cols-4">
              {METRICS.map((m) => (
                <div
                  key={m.label}
                  className="rounded-xl border border-white/[0.07] bg-white/[0.025] px-3.5 py-2.5"
                >
                  <div className="font-mono text-[10px] uppercase tracking-wider text-zinc-500">
                    {m.label}
                  </div>
                  <div className="mt-0.5 flex items-center gap-1.5 font-mono text-xs font-semibold text-zinc-200">
                    <Cpu className="h-3 w-3 text-[#8b6bff] shrink-0" />
                    <span className="truncate">{m.value}</span>
                  </div>
                </div>
              ))}
            </div>
          </motion.div>
        </div>
      </div>
    </section>
  );
}
