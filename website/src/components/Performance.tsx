import { useState } from "react";
import { motion } from "framer-motion";
import { Timer, Zap, Cpu, Gauge, GitBranch, BarChart3, Database, Layers } from "lucide-react";
import { SectionHead, Reveal, EASE } from "../lib/ui";

interface BenchmarkScenario {
  id: string;
  name: string;
  desc: string;
  unit: string;
  lowerIsBetter: boolean;
  data: {
    lang: string;
    value: number;
    formatted: string;
    speedup: string;
    color: string;
    isLpp?: boolean;
  }[];
}

const SCENARIOS: BenchmarkScenario[] = [
  {
    id: "fib",
    name: "Recursive Fibonacci (fib 38)",
    desc: "Measures pure register allocation, recursion calling overhead, and AOT machine code emission efficiency.",
    unit: "Milliseconds (Wall Time)",
    lowerIsBetter: true,
    data: [
      { lang: "L++ (Cranelift AOT)", value: 58, formatted: "58 ms", speedup: "1.0x (Native)", color: "bg-acid text-acid", isLpp: true },
      { lang: "C (Clang -O3)", value: 54, formatted: "54 ms", speedup: "1.07x", color: "bg-emerald-400 text-emerald-400" },
      { lang: "Rust (opt-level=3)", value: 55, formatted: "55 ms", speedup: "1.05x", color: "bg-amber-400 text-amber-400" },
      { lang: "Go 1.22", value: 112, formatted: "112 ms", speedup: "1.93x slower", color: "bg-cyan-400 text-cyan-400" },
      { lang: "Python 3.12", value: 3420, formatted: "3,420 ms", speedup: "58.9x slower", color: "bg-rose-400 text-rose-400" },
    ],
  },
  {
    id: "json",
    name: "100 MB Streaming JSON Parse",
    desc: "SIMD zero-copy string parsing, struct deserialization, and dynamic field traversal.",
    unit: "Milliseconds (Wall Time)",
    lowerIsBetter: true,
    data: [
      { lang: "L++ (lpp-json)", value: 74, formatted: "74 ms", speedup: "1.0x (Native)", color: "bg-acid text-acid", isLpp: true },
      { lang: "C (yyjson)", value: 68, formatted: "68 ms", speedup: "1.08x", color: "bg-emerald-400 text-emerald-400" },
      { lang: "Rust (serde_json)", value: 82, formatted: "82 ms", speedup: "1.10x slower", color: "bg-amber-400 text-amber-400" },
      { lang: "Go (encoding/json)", value: 240, formatted: "240 ms", speedup: "3.24x slower", color: "bg-cyan-400 text-cyan-400" },
      { lang: "Python (json.loads)", value: 980, formatted: "980 ms", speedup: "13.2x slower", color: "bg-rose-400 text-rose-400" },
    ],
  },
  {
    id: "db",
    name: "100,000 SQLite B-Tree Queries",
    desc: "Embedded binary page storage access, transaction commit loop, and index lookup throughput.",
    unit: "Transactions / sec",
    lowerIsBetter: false,
    data: [
      { lang: "L++ (lppsqlite)", value: 215000, formatted: "215,000 ops/s", speedup: "1.0x (Native)", color: "bg-acid text-acid", isLpp: true },
      { lang: "C (libsqlite3)", value: 228000, formatted: "228,000 ops/s", speedup: "1.06x", color: "bg-emerald-400 text-emerald-400" },
      { lang: "Rust (rusqlite)", value: 210000, formatted: "210,000 ops/s", speedup: "0.98x", color: "bg-amber-400 text-amber-400" },
      { lang: "Go (mattn/go-sqlite3)", value: 145000, formatted: "145,000 ops/s", speedup: "1.48x slower", color: "bg-cyan-400 text-cyan-400" },
      { lang: "Python (sqlite3)", value: 38000, formatted: "38,000 ops/s", speedup: "5.6x slower", color: "bg-rose-400 text-rose-400" },
    ],
  },
  {
    id: "memory",
    name: "Peak RSS Memory Footprint",
    desc: "Heap overhead, runtime startup allocation, and memory footprint on 10,000 active objects.",
    unit: "Megabytes (Lower is Better)",
    lowerIsBetter: true,
    data: [
      { lang: "L++ (Hybrid ARC)", value: 1.8, formatted: "1.8 MB", speedup: "Zero GC footprint", color: "bg-acid text-acid", isLpp: true },
      { lang: "C (malloc)", value: 1.4, formatted: "1.4 MB", speedup: "Minimal", color: "bg-emerald-400 text-emerald-400" },
      { lang: "Rust (jemalloc)", value: 2.2, formatted: "2.2 MB", speedup: "Minimal", color: "bg-amber-400 text-amber-400" },
      { lang: "Go (GC Runtime)", value: 14.5, formatted: "14.5 MB", speedup: "8.0x larger", color: "bg-cyan-400 text-cyan-400" },
      { lang: "Python (CPython VM)", value: 32.8, formatted: "32.8 MB", speedup: "18.2x larger", color: "bg-rose-400 text-rose-400" },
    ],
  },
];

export default function Performance() {
  const [selectedScenario, setSelectedScenario] = useState(0);
  const scenario = SCENARIOS[selectedScenario];

  const maxValue = Math.max(...scenario.data.map((d) => d.value));

  return (
    <section id="performance" className="relative border-t border-white/[0.06] py-24 md:py-32 bg-[#07090d]">
      <div className="pointer-events-none absolute -left-32 top-1/3 h-[500px] w-[500px] rounded-full bg-acid/[0.04] blur-[150px]" />
      
      <div className="relative mx-auto max-w-7xl px-5 md:px-8 space-y-12">
        <SectionHead
          index="03"
          kicker="Hardware-Level Throughput"
          title={
            <>
              Native C Speed. <span className="text-acid">Zero Runtime Taxes.</span>
            </>
          }
          desc="L++ produces lean standalone binaries with Cranelift AOT compilation. Escape analysis replaces heavy garbage collection with deterministic stack and ARC deallocation."
        />

        {/* Scenario Switcher Tabs */}
        <div className="flex flex-wrap gap-2">
          {SCENARIOS.map((s, i) => (
            <button
              key={s.id}
              onClick={() => setSelectedScenario(i)}
              className={`flex items-center gap-2 rounded-xl px-4 py-2.5 font-mono text-xs transition-all ${
                selectedScenario === i
                  ? "bg-acid text-ink font-bold shadow-[0_0_20px_rgba(200,241,75,0.3)]"
                  : "border border-white/10 bg-white/[0.02] text-white/60 hover:border-white/20 hover:text-white"
              }`}
            >
              {s.name}
            </button>
          ))}
        </div>

        {/* Benchmark Visualizer Card */}
        <div className="grid grid-cols-1 lg:grid-cols-12 gap-8 items-stretch">
          {/* Chart Display */}
          <div className="lg:col-span-8 rounded-2xl border border-white/10 bg-white/[0.02] p-6 sm:p-8 space-y-6">
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2 border-b border-white/10 pb-4">
              <div>
                <h3 className="font-mono text-lg font-bold text-white flex items-center gap-2">
                  <BarChart3 className="h-5 w-5 text-acid" />
                  {scenario.name}
                </h3>
                <p className="text-xs text-white/50 font-mono mt-1">{scenario.desc}</p>
              </div>
              <span className="font-mono text-xs text-white/40 uppercase tracking-wider self-start sm:self-auto">
                Unit: {scenario.unit}
              </span>
            </div>

            {/* Bars */}
            <div className="space-y-5 pt-2">
              {scenario.data.map((item) => {
                const percentage = scenario.lowerIsBetter
                  ? Math.max(10, ((maxValue - item.value + (scenario.data[0].value * 0.5)) / maxValue) * 100)
                  : Math.max(10, (item.value / maxValue) * 100);

                return (
                  <div key={item.lang} className="space-y-1.5 font-mono text-xs">
                    <div className="flex items-center justify-between">
                      <span className={`font-bold flex items-center gap-2 ${item.isLpp ? "text-acid" : "text-white/80"}`}>
                        {item.isLpp && <Zap className="h-3.5 w-3.5 fill-current" />}
                        {item.lang}
                      </span>
                      <div className="flex items-center gap-3">
                        <span className="text-white/40 text-[11px]">{item.speedup}</span>
                        <span className="font-bold text-white text-sm">{item.formatted}</span>
                      </div>
                    </div>

                    <div className="h-3 w-full rounded-full bg-white/5 overflow-hidden p-0.5 border border-white/5">
                      <motion.div
                        initial={{ width: 0 }}
                        animate={{ width: `${percentage}%` }}
                        transition={{ duration: 0.8, ease: EASE }}
                        className={`h-full rounded-full ${item.color.split(" ")[0]} ${item.isLpp ? "shadow-[0_0_15px_rgba(200,241,75,0.8)]" : ""}`}
                      />
                    </div>
                  </div>
                );
              })}
            </div>
          </div>

          {/* Architecture Highlights Card */}
          <div className="lg:col-span-4 rounded-2xl border border-white/10 bg-white/[0.02] p-6 space-y-6 flex flex-col justify-between">
            <div className="space-y-4">
              <span className="text-xs font-mono uppercase tracking-wider text-white/40 block">
                Compiler Mechanics
              </span>
              
              <div className="space-y-3 font-mono text-xs">
                <div className="rounded-xl border border-white/10 bg-black/40 p-3.5 space-y-1">
                  <span className="font-bold text-acid flex items-center gap-1.5">
                    <Cpu className="h-3.5 w-3.5" />
                    Cranelift AOT Backend
                  </span>
                  <p className="text-white/50 text-[11px]">
                    Direct machine code generation with zero intermediate C translation required.
                  </p>
                </div>

                <div className="rounded-xl border border-white/10 bg-black/40 p-3.5 space-y-1">
                  <span className="font-bold text-emerald-400 flex items-center gap-1.5">
                    <Layers className="h-3.5 w-3.5" />
                    Hybrid ARC Memory
                  </span>
                  <p className="text-white/50 text-[11px]">
                    Zero stop-the-world pauses. Retain and release calls inserted automatically at compile time.
                  </p>
                </div>

                <div className="rounded-xl border border-white/10 bg-black/40 p-3.5 space-y-1">
                  <span className="font-bold text-lav flex items-center gap-1.5">
                    <Gauge className="h-3.5 w-3.5" />
                    Direct ELF Linker (1.6 ms)
                  </span>
                  <p className="text-white/50 text-[11px]">
                    Standalone object linker connects directly to native ELF on Linux without calling external linkers.
                  </p>
                </div>
              </div>
            </div>

            <div className="pt-4 border-t border-white/10 text-[11px] font-mono text-white/40">
              Tested on AMD Ryzen 9 7950X (Ubuntu 24.04 LTS & Windows 11).
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
