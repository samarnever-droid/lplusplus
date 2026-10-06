import { useState } from "react";
import { motion } from "framer-motion";
import {
  Terminal,
  Check,
  Copy,
  Sparkles,
  Zap,
  ShieldCheck,
  Package,
  Layers,
  ArrowUpRight,
  Play,
  FileCode,
  Cpu,
} from "lucide-react";
import { Code } from "../lib/highlight";
import { EASE } from "../lib/ui";

interface CodeTab {
  id: string;
  name: string;
  badge: string;
  code: string;
  stdout: string;
  ir: string;
  escapeLog: { name: string; type: string; dest: string; color: string }[];
}

const TABS: CodeTab[] = [
  {
    id: "fib",
    name: "fibonacci.lpp",
    badge: "Direct AOT",
    code: `def fib(n: Int) -> Int:
    if n < 2:
        return n
    return fib(n - 1) + fib(n - 2)

def main():
    result := fib(35)
    print("fib(35) =", result)`,
    stdout: `[L++] Compiling with Cranelift AOT...
[L++] Native executable linked
fib(35) = 9227465`,
    ir: `function u0:0(i64) -> i64 fast {
block0(v0: i64):
    v1 = iconst.i64 2
    v2 = icmp slt v0, v1
    brif v2, block1, block2
block1:
    return v0
block2:
    v3 = iadd_imm v0, -1
    v4 = call u0:0(v3)
    v5 = iadd_imm v0, -2
    v6 = call u0:0(v5)
    v7 = iadd v4, v6
    return v7
}`,
    escapeLog: [
      { name: "n", type: "Int · scalar", dest: "CPU Register / Stack", color: "text-acid" },
      { name: "result", type: "Int · scalar", dest: "Stack (Zero Heap)", color: "text-acid" },
    ],
  },
  {
    id: "memory",
    name: "hybrid_memory.lpp",
    badge: "ARC & Escape",
    code: `struct Item:
    id: Int
    name: Str

def create_item(id: Int, name: Str) -> Item:
    item := Item(id, name)
    return item  # escapes this frame

def main():
    item := create_item(101, "Server Cluster")
    print_str(item.name)`,
    stdout: `[L++] ownership: item -> Owned
[L++] ownership balance verified
Server Cluster`,
    ir: `ownership-plan v1
  item: frame -> owned
  contains: Item -> Str
  balance: verified`,
    escapeLog: [
      { name: "id", type: "Int · scalar", dest: "Frame", color: "text-acid" },
      { name: "item", type: "struct · escapes", dest: "Owned", color: "text-lav" },
    ],
  },
  {
    id: "match",
    name: "result_match.lpp",
    badge: "Typed Match",
    code: `enum Result:
    Ready(message: Str)
    Failed(code: Int)

def report(result: Result):
    match result:
        Ready(message):
            print_str(message)
        Failed(code):
            print(code)

def main():
    report(Result.Ready("ready"))`,
    stdout: `[L++] exhaustive match verified
ready`,
    ir: `switch_variant result
  Ready(message) -> print_str(message)
  Failed(code)   -> print(code)`,
    escapeLog: [
      { name: "result", type: "enum · local", dest: "Frame", color: "text-acid" },
      { name: "message", type: "Str · payload", dest: "Borrowed call", color: "text-lav" },
    ],
  },
  {
    id: "wasm",
    name: "portable_lists.lpp",
    badge: "Native + WASM",
    code: `def sum(values: List[Int]) -> Int:
    mut total := 0
    for value in values:
        total = total + value
    return total

def main():
    values := [10, 20, 30]
    print(sum(values))`,
    stdout: `[L++] emitted validated wasm32-wasip1 module
60`,
    ir: `values = list [10, 20, 30]
for value in values:
  total = total + value
return total`,
    escapeLog: [
      { name: "values", type: "List[Int]", dest: "Owned", color: "text-lav" },
      { name: "total", type: "Int · scalar", dest: "Frame", color: "text-acid" },
    ],
  },
];

export default function Hero() {
  const [activeTab, setActiveTab] = useState(0);
  const [viewMode, setViewMode] = useState<"code" | "ir" | "stdout">("code");
  const [isRunning, setIsRunning] = useState(false);
  const [copiedInstall, setCopiedInstall] = useState<string | null>(null);
  const [osTab, setOsTab] = useState<"curl" | "powershell" | "cargo">("curl");

  const tab = TABS[activeTab];

  const handleRun = () => {
    setIsRunning(true);
    setViewMode("stdout");
    setTimeout(() => {
      setIsRunning(false);
    }, 450);
  };

  const copyText = (txt: string, id: string) => {
    navigator.clipboard.writeText(txt);
    setCopiedInstall(id);
    setTimeout(() => setCopiedInstall(null), 2000);
  };

  const installCommands = {
    curl: "git clone --depth 1 --branch v0.1 https://github.com/samarnever-droid/lplusplus.git && cd lplusplus && LPP_FROM_SOURCE=1 sh install.sh",
    powershell: "git clone --depth 1 --branch v0.1 https://github.com/samarnever-droid/lplusplus.git; cd lplusplus; $env:LPP_FROM_SOURCE='1'; .\\install.ps1",
    cargo: "cargo install --locked --git https://github.com/samarnever-droid/lplusplus --tag v0.1",
  };

  return (
    <section id="top" className="relative overflow-hidden pt-28 pb-20 md:pt-36 md:pb-28">
      {/* Background Ambient Glows */}
      <div className="pointer-events-none absolute -top-40 left-1/2 h-[600px] w-[1000px] -translate-x-1/2 rounded-full bg-acid/[0.08] blur-[150px]" />
      <div className="pointer-events-none absolute -right-40 top-1/3 h-[500px] w-[500px] rounded-full bg-lav/[0.06] blur-[140px]" />
      <div className="pointer-events-none absolute -left-40 bottom-10 h-[450px] w-[450px] rounded-full bg-aqua/[0.05] blur-[130px]" />

      <div className="relative mx-auto max-w-7xl px-5 md:px-8">
        <div className="grid items-center gap-12 lg:grid-cols-12">
          {/* Left Column: Value Prop & Headlines */}
          <div className="lg:col-span-6 space-y-7">
            {/* Version Badge */}
            <motion.div
              initial={{ opacity: 0, y: 15 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ duration: 0.7, ease: EASE }}
              className="inline-flex items-center gap-3 rounded-full border border-white/15 bg-white/[0.03] py-1.5 pl-2 pr-4 shadow-xl backdrop-blur-md"
            >
              <span className="flex items-center gap-1.5 rounded-full bg-acid/20 px-3 py-0.5 font-mono text-[11px] font-bold uppercase tracking-wider text-acid border border-acid/30">
                <span className="h-2 w-2 rounded-full bg-acid animate-pulse" />
                L++ v0.1
              </span>
              <span className="font-mono text-xs text-white/70">
                Readable &bull; Native &bull; Safety-first
              </span>
            </motion.div>

            {/* Typography Master Headline */}
            <div className="space-y-2">
              <h1 className="font-mono text-4xl sm:text-6xl font-black tracking-tight text-white leading-[1.08]">
                Readable by design.
                <br />
                <span className="text-acid">Native by default.</span>
                <br />
                <span className="text-white/55">Safety engineered in.</span>
              </h1>
            </div>

            <p className="text-base sm:text-lg leading-relaxed text-white/65 max-w-xl font-sans">
              L++ brings Python-like clarity to a statically typed, ahead-of-time language. Ownership analysis
              places values as frame, owned, or shared and catches common ownership mistakes before code
              generation—without a tracing garbage collector.
            </p>

            {/* Quick OS Install Switcher */}
            <div className="space-y-2 pt-2">
              <div className="flex items-center gap-2">
                {(["curl", "powershell", "cargo"] as const).map((os) => (
                  <button
                    key={os}
                    onClick={() => setOsTab(os)}
                    className={`rounded-lg px-3 py-1 font-mono text-xs font-semibold uppercase tracking-wider transition-all ${
                      osTab === os
                        ? "bg-white/15 text-acid border border-acid/40"
                        : "text-white/50 hover:text-white/90"
                    }`}
                  >
                    {os === "curl" ? "Linux / macOS" : os === "powershell" ? "Windows" : "Cargo"}
                  </button>
                ))}
              </div>

              <div className="flex items-center justify-between gap-3 rounded-xl border border-white/15 bg-black/80 p-3.5 shadow-2xl backdrop-blur-xl">
                <div className="flex items-center gap-2 overflow-x-auto text-xs font-mono text-white/90">
                  <Terminal className="h-4 w-4 shrink-0 text-acid" />
                  <code className="whitespace-nowrap">{installCommands[osTab]}</code>
                </div>
                <button
                  onClick={() => copyText(installCommands[osTab], `install-${osTab}`)}
                  className="flex items-center gap-1.5 rounded-lg bg-white/10 px-3 py-1.5 font-mono text-xs text-white hover:bg-white/20 shrink-0"
                >
                  {copiedInstall === `install-${osTab}` ? (
                    <Check className="h-3.5 w-3.5 text-emerald-400" />
                  ) : (
                    <Copy className="h-3.5 w-3.5" />
                  )}
                  {copiedInstall === `install-${osTab}` ? "Copied" : "Copy"}
                </button>
              </div>
            </div>

            {/* Call to Actions */}
            <div className="flex flex-wrap items-center gap-4 pt-2">
              <a
                href={`${import.meta.env.BASE_URL}academy.html`}
                className="flex items-center gap-2 rounded-xl bg-acid px-6 py-3.5 font-mono text-sm font-bold text-ink transition-all hover:brightness-110 shadow-[0_0_30px_rgba(200,241,75,0.35)]"
              >
                <Sparkles className="h-4 w-4" />
                Launch Academy Playground
              </a>
              <a
                href={`${import.meta.env.BASE_URL}packages.html`}
                className="flex items-center gap-2 rounded-xl border border-white/15 bg-white/5 px-6 py-3.5 font-mono text-sm font-semibold text-white hover:border-acid/40 hover:text-acid transition-all"
              >
                <Package className="h-4 w-4" />
                Browse 16 Packages
                <ArrowUpRight className="h-4 w-4" />
              </a>
            </div>

            {/* Live Telemetry Tickers */}
            <div className="grid grid-cols-2 sm:grid-cols-4 gap-3 pt-4 border-t border-white/10 text-xs font-mono">
              <div className="rounded-xl border border-white/10 bg-white/[0.02] p-3 space-y-1">
                <span className="text-white/40 block text-[10px] uppercase">Compilation</span>
                <span className="font-bold text-acid text-sm sm:text-base">Native AOT</span>
              </div>
              <div className="rounded-xl border border-white/10 bg-white/[0.02] p-3 space-y-1">
                <span className="text-white/40 block text-[10px] uppercase">Ownership</span>
                <span className="font-bold text-emerald-400 text-[11px] sm:text-xs">Frame · Owned · Shared</span>
              </div>
              <div className="rounded-xl border border-white/10 bg-white/[0.02] p-3 space-y-1">
                <span className="text-white/40 block text-[10px] uppercase">WASM Corpus</span>
                <span className="font-bold text-white text-sm sm:text-base">24 / 24</span>
              </div>
              <div className="rounded-xl border border-white/10 bg-white/[0.02] p-3 space-y-1">
                <span className="text-white/40 block text-[10px] uppercase">Tracing GC</span>
                <span className="font-bold text-lav text-sm sm:text-base">None</span>
              </div>
            </div>
          </div>

          {/* Right Column: Interactive Code Workbench */}
          <div className="lg:col-span-6 space-y-4">
            <div className="overflow-hidden rounded-2xl border border-white/15 bg-[#0b0e14] shadow-[0_30px_90px_rgba(0,0,0,0.8)] backdrop-blur-xl">
              {/* Tab Bar */}
              <div className="flex items-center justify-between border-b border-white/10 bg-black/60 px-4 py-2.5">
                <div className="flex items-center gap-1.5 overflow-x-auto">
                  {TABS.map((t, i) => (
                    <button
                      key={t.id}
                      onClick={() => {
                        setActiveTab(i);
                        setViewMode("code");
                      }}
                      className={`flex items-center gap-1.5 rounded-lg px-3 py-1.5 font-mono text-xs transition-all ${
                        activeTab === i
                          ? "bg-white/15 text-white font-bold border border-white/20"
                          : "text-white/45 hover:text-white hover:bg-white/5"
                      }`}
                    >
                      <span>{t.name}</span>
                      <span className="rounded bg-acid/15 px-1.5 py-0.2 text-[9px] text-acid font-semibold uppercase">
                        {t.badge}
                      </span>
                    </button>
                  ))}
                </div>

                <button
                  onClick={handleRun}
                  disabled={isRunning}
                  className="flex items-center gap-1.5 rounded-lg bg-acid px-3 py-1.5 font-mono text-xs font-bold text-ink hover:brightness-110 shadow-md shrink-0"
                >
                  <Play className={`h-3.5 w-3.5 ${isRunning ? "animate-spin" : "fill-current"}`} />
                  {isRunning ? "Running..." : "Run L++"}
                </button>
              </div>

              {/* View Mode Toggle: Source / IR / Output */}
              <div className="flex items-center justify-between border-b border-white/10 bg-black/30 px-4 py-2 text-[11px] font-mono text-white/50">
                <div className="flex items-center gap-2">
                  <button
                    onClick={() => setViewMode("code")}
                    className={`flex items-center gap-1 px-2 py-0.5 rounded ${
                      viewMode === "code" ? "bg-white/15 text-white font-bold" : "hover:text-white"
                    }`}
                  >
                    <FileCode className="h-3 w-3" />
                    Source Code
                  </button>
                  <button
                    onClick={() => setViewMode("ir")}
                    className={`flex items-center gap-1 px-2 py-0.5 rounded ${
                      viewMode === "ir" ? "bg-white/15 text-white font-bold" : "hover:text-white"
                    }`}
                  >
                    <Cpu className="h-3 w-3" />
                    Cranelift IR
                  </button>
                  <button
                    onClick={() => setViewMode("stdout")}
                    className={`flex items-center gap-1 px-2 py-0.5 rounded ${
                      viewMode === "stdout" ? "bg-white/15 text-acid font-bold" : "hover:text-white"
                    }`}
                  >
                    <Terminal className="h-3 w-3" />
                    Execution Output
                  </button>
                </div>

                <span className="text-[10px] text-white/30 hidden sm:inline">
                  Direct Target: x86-64 Native
                </span>
              </div>

              {/* Editor Window */}
              <div className="relative min-h-[300px] p-5 font-mono text-[13px] leading-[1.8] overflow-x-auto bg-[#07090d]">
                {viewMode === "code" && (
                  <pre className="whitespace-pre">
                    <Code src={tab.code} />
                  </pre>
                )}

                {viewMode === "ir" && (
                  <pre className="whitespace-pre text-white/75 text-[12px] leading-relaxed">
                    {tab.ir}
                  </pre>
                )}

                {viewMode === "stdout" && (
                  <pre className="whitespace-pre text-acid text-[12px] leading-relaxed font-mono">
                    {tab.stdout}
                  </pre>
                )}
              </div>

              {/* Escape Analysis HUD Footer */}
              <div className="border-t border-white/10 bg-black/60 p-4 space-y-2">
                <div className="flex items-center justify-between text-[11px] font-mono uppercase tracking-wider text-white/40">
                  <span className="flex items-center gap-1.5">
                    <Layers className="h-3.5 w-3.5 text-acid" />
                    Escape Analysis Side-Table (0.3 ms)
                  </span>
                  <span className="text-acid">Zero GC Pause</span>
                </div>
                <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
                  {tab.escapeLog.map((log) => (
                    <div
                      key={log.name}
                      className="flex items-center justify-between rounded-lg border border-white/10 bg-white/[0.02] px-3 py-1.5 text-xs font-mono"
                    >
                      <span className="text-white font-bold">{log.name}</span>
                      <span className="text-white/40 text-[10px]">{log.type}</span>
                      <span className={`font-semibold text-[11px] ${log.color}`}>{log.dest}</span>
                    </div>
                  ))}
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
