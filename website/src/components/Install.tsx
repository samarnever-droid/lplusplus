import { useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Copy, Check, TerminalSquare, Hammer, FolderGit2, Play } from "lucide-react";
import { SectionHead, Reveal, EASE } from "../lib/ui";

const TABS = [
  {
    id: "windows",
    label: "Windows (PowerShell)",
    copy: "irm https://samarnever-droid.github.io/lplusplus/install.ps1 | iex",
    lines: [
      { p: "PS>", t: "irm https://samarnever-droid.github.io/lplusplus/install.ps1 | iex", c: "text-white" },
      { p: "", t: "  downloading lpp.exe ................... ok", c: "text-white/45" },
      { p: "", t: "  downloading lpp_runtime.c ............. ok", c: "text-white/45" },
      { p: "", t: "  precompiling lpp_runtime.obj .......... ok", c: "text-white/45" },
      { p: "", t: "  path adding ~/.lpp/bin to PATH ........ ok", c: "text-white/45" },
      { p: "", t: "", c: "" },
      { p: "PS>", t: "lpp -h", c: "text-white" },
      { p: "", t: "L++ Compiler v0.1.0", c: "text-acid" },
    ],
  },
  {
    id: "unix",
    label: "macOS / Linux",
    copy: "curl -sSfL https://samarnever-droid.github.io/lplusplus/install.sh | sh",
    lines: [
      { p: "$", t: "curl -sSfL https://samarnever-droid.github.io/lplusplus/install.sh | sh", c: "text-white" },
      { p: "", t: "  downloading lpp_runtime.c ............. ok", c: "text-white/45" },
      { p: "", t: "  compiling lpp from source ............. ok", c: "text-white/45" },
      { p: "", t: "  precompiling lpp_runtime.o ............ ok", c: "text-white/45" },
      { p: "", t: "", c: "" },
      { p: "$", t: "lpp -h", c: "text-white" },
      { p: "", t: "L++ Compiler v0.1.0", c: "text-acid" },
    ],
  },
];

const STEPS = [
  {
    icon: TerminalSquare,
    title: "Run the installer",
    body: "Open PowerShell in the project root and execute .\\install.ps1. It builds the release compiler and stages everything under %USERPROFILE%\\.lpp.",
  },
  {
    icon: FolderGit2,
    title: "Restart your terminal",
    body: "The installer adds ~/.lpp/bin to your user PATH. Reopen your shell or IDE and the global lpp command is live.",
  },
  {
    icon: Play,
    title: "Choose file or package mode",
    body: "Use lpp emit file.lpp for source artifacts, lpp build for packages, or LPP_LINKER=direct lpp build on verified Linux x86-64 projects.",
  },
];

export default function Install() {
  const [tab, setTab] = useState(0);
  const [copied, setCopied] = useState(false);
  const current = TABS[tab];

  const doCopy = async () => {
    try {
      await navigator.clipboard.writeText(current.copy);
    } catch {
      /* clipboard unavailable */
    }
    setCopied(true);
    setTimeout(() => setCopied(false), 1600);
  };

  return (
    <section id="install" className="relative border-t border-white/[0.06] py-28 md:py-36">
      <div className="pointer-events-none absolute bottom-0 left-1/2 h-[420px] w-[900px] -translate-x-1/2 rounded-full bg-acid/[0.06] blur-[140px]" />
      <div className="relative mx-auto max-w-7xl px-5 md:px-8">
        <SectionHead
          index="07"
          kicker="Zero-friction toolchain"
          title={
            <>
              From clone to native binary{" "}
              <span className="text-acid">in one command.</span>
            </>
          }
          desc="A premium toolchain wrapper installs the compiler globally: binary, runtime library, CLI shim and PATH wiring — handled for you."
        />

        <div className="mt-14 grid gap-6 lg:grid-cols-[0.9fr_1.1fr]">
          <div className="space-y-4">
            {STEPS.map((s, i) => (
              <Reveal key={s.title} delay={i * 0.09}>
                <div className="group flex gap-5 rounded-2xl border border-white/[0.08] bg-panel p-6 transition-colors hover:border-acid/25">
                  <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl border border-acid/25 bg-acid/[0.07]">
                    <s.icon className="h-5 w-5 text-acid" />
                  </div>
                  <div>
                    <p className="font-mono text-[10px] text-white/30">step 0{i + 1}</p>
                    <h3 className="mt-1 font-display text-lg font-semibold tracking-tight text-white">
                      {s.title}
                    </h3>
                    <p className="mt-1.5 text-[14px] leading-relaxed text-white/45">{s.body}</p>
                  </div>
                </div>
              </Reveal>
            ))}
          </div>

          <Reveal delay={0.15}>
            <div className="overflow-hidden rounded-2xl border border-white/10 bg-[#0a0c0f] shadow-[0_40px_90px_-25px_rgb(0_0_0/0.9)]">
              <div className="flex items-center gap-2 border-b border-white/[0.07] bg-white/[0.025] px-4 py-3">
                <span className="h-2.5 w-2.5 rounded-full bg-[#ff5f57]" />
                <span className="h-2.5 w-2.5 rounded-full bg-[#febc2e]" />
                <span className="h-2.5 w-2.5 rounded-full bg-[#28c840]" />
                <div className="ml-4 flex gap-1">
                  {TABS.map((t, i) => (
                    <button
                      key={t.id}
                      onClick={() => setTab(i)}
                      className={`rounded-md px-3 py-1.5 font-mono text-[11px] transition-colors ${
                        i === tab ? "bg-acid/15 text-acid" : "text-white/40 hover:text-white/70"
                      }`}
                    >
                      {t.label}
                    </button>
                  ))}
                </div>
                <button
                  onClick={doCopy}
                  className="ml-auto flex items-center gap-1.5 rounded-md border border-white/10 px-2.5 py-1.5 font-mono text-[10.5px] text-white/50 transition-colors hover:border-acid/40 hover:text-acid"
                >
                  {copied ? <Check className="h-3 w-3 text-acid" /> : <Copy className="h-3 w-3" />}
                  {copied ? "copied" : "copy"}
                </button>
              </div>
              <div className="min-h-[380px] p-5 font-mono text-[12.5px] leading-[1.9]">
                <AnimatePresence mode="wait">
                  <motion.div
                    key={current.id}
                    initial={{ opacity: 0, y: 10 }}
                    animate={{ opacity: 1, y: 0 }}
                    exit={{ opacity: 0 }}
                    transition={{ duration: 0.35, ease: EASE }}
                  >
                    {current.lines.map((l, i) => (
                      <p key={i} className={l.c}>
                        {l.p && <span className="mr-2 select-none text-lav">{l.p}</span>}
                        {l.t || "\u00A0"}
                      </p>
                    ))}
                    <p>
                      <span className="mr-2 select-none text-lav">PS&gt;</span>
                      <span className="ml-0.5 inline-block h-[14px] w-[8px] translate-y-[2px] bg-acid animate-blink" />
                    </p>
                  </motion.div>
                </AnimatePresence>
              </div>
            </div>
            <div className="mt-4 flex items-center gap-3 rounded-xl border border-white/[0.08] bg-panel px-5 py-4">
              <Hammer className="h-4 w-4 shrink-0 text-acid" />
              <p className="font-mono text-[11.5px] leading-relaxed text-white/40">
                requires Windows + MSVC build tools today — the C transpiler backend targets GCC /
                Clang toolchains next.
              </p>
            </div>
          </Reveal>
        </div>
      </div>
    </section>
  );
}
