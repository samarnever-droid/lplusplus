import { useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Layers, Boxes, Network, Lock, MoveRight, ScanSearch } from "lucide-react";
import { SectionHead, Reveal, EASE } from "../lib/ui";
import { CodeBlock } from "../lib/highlight";

type Target = "stack" | "heap" | "arena";

interface Rule {
  id: number;
  title: string;
  short: string;
  code: string;
  highlight: number[];
  target: Target;
  varName: string;
  stayed: string[];
  verdict: string;
}

const RULES: Rule[] = [
  {
    id: 1,
    title: "Returned by Reference",
    short: "A local returned to the caller escapes its stack frame.",
    code: `struct Item:\n    value: Int\n\ndef create_item() -> Item:\n    item := Item()\n    return item   # escapes its frame`,
    highlight: [6],
    target: "heap",
    varName: "item",
    stayed: [],
    verdict:
      "item outlives its own frame, so the compiler ref-counts it on the Managed Heap. Returning a scalar copy like box.count never triggers this.",
  },
  {
    id: 2,
    title: "Closure Capture",
    short: "Captured by a closure that outlives its scope.",
    code: `def process():\n    multiplier := 5\n    config := Config()\n    callback := fn(x) -> Int:\n        print(config)          # captured\n        return x * multiplier  # cloned by value`,
    highlight: [5],
    target: "heap",
    varName: "config",
    stayed: ["multiplier"],
    verdict:
      "config is captured by an escaping closure → Managed Heap. multiplier is an immutable scalar — cloned by value, stays on the stack for free.",
  },
  {
    id: 3,
    title: "Unbounded Containers",
    short: "Stored in a container with a dynamic lifetime.",
    code: `def build_list() -> Void:\n    cap := 8\n    node := Node()\n    my_list := [node]   # lifetime becomes dynamic`,
    highlight: [4],
    target: "heap",
    varName: "node",
    stayed: ["cap"],
    verdict:
      "A list's lifetime is unbounded — it can grow, shrink, and travel anywhere. Anything stored inside is promoted to the Managed Heap.",
  },
  {
    id: 4,
    title: "Concurrency Boundary",
    short: "Captured by a spawn closure crossing a thread.",
    code: `def parallel_work() -> Void:\n    readonly := 100\n    mut shared := 0\n    spawn fn() -> Void:\n        print(readonly, shared)`,
    highlight: [4],
    target: "heap",
    varName: "shared",
    stayed: ["readonly"],
    verdict:
      "shared is mutable state crossing a thread boundary → Managed Heap for safe sharing. readonly is immutable: copied straight onto the new thread's stack.",
  },
  {
    id: 5,
    title: "Self-Referential Structs",
    short: "A struct containing its own type becomes graph-shaped.",
    code: `struct Node:\n    value: Int\n    next: Node    # type-level cycle\n\ndef main():\n    depth := 0\n    node := Node()`,
    highlight: [3, 7],
    target: "arena",
    varName: "node",
    stayed: ["depth"],
    verdict:
      "Node references its own type — linked lists, trees, graphs. Instances are bulk-allocated in an Arena: one blazing-fast region, freed in a single shot.",
  },
];

const ZONES: Record<
  Target,
  { icon: typeof Layers; name: string; sub: string; desc: string; text: string; border: string; bg: string; dot: string }
> = {
  stack: {
    icon: Layers,
    name: "Stack",
    sub: "Value storage",
    desc: "zero-cost · frame-bound",
    text: "text-acid",
    border: "border-acid/45",
    bg: "bg-acid/[0.06]",
    dot: "bg-acid",
  },
  heap: {
    icon: Boxes,
    name: "Managed Heap",
    sub: "ARC",
    desc: "automatic ref-counting",
    text: "text-lav",
    border: "border-lav/45",
    bg: "bg-lav/[0.06]",
    dot: "bg-lav",
  },
  arena: {
    icon: Network,
    name: "Arena",
    sub: "bulk allocator",
    desc: "graphs · freed in one shot",
    text: "text-aqua",
    border: "border-aqua/45",
    bg: "bg-aqua/[0.06]",
    dot: "bg-aqua",
  },
};

export default function MemoryModel() {
  const [active, setActive] = useState(0);
  const rule = RULES[active];

  return (
    <section id="memory" className="relative border-t border-white/[0.06] py-28 md:py-36">
      <div className="pointer-events-none absolute left-1/2 top-0 h-[420px] w-[820px] -translate-x-1/2 rounded-full bg-lav/[0.05] blur-[130px]" />
      <div className="relative mx-auto max-w-7xl px-5 md:px-8">
        <SectionHead
          index="02"
          kicker="The magic — hybrid memory model"
          title={
            <>
              You write code. The compiler{" "}
              <span className="text-acid">decides where it lives.</span>
            </>
          }
          desc="Every binding starts as a zero-cost stack value. A semantic pass — Escape Analysis — checks five rules and monotonically promotes escaping values to the ARC Managed Heap or an Arena. No Box, no Rc, no &, no *. Ever."
        />

        {/* promotion ladder */}
        <Reveal delay={0.1} className="mt-10">
          <div className="flex flex-wrap items-center gap-x-4 gap-y-3 rounded-2xl border border-white/[0.08] bg-panel px-5 py-4">
            <span className="font-mono text-[10px] uppercase tracking-[0.25em] text-white/35">
              promotion ladder
            </span>
            <div className="flex flex-wrap items-center gap-x-3 gap-y-2 font-mono text-[12px]">
              <span className="flex items-center gap-2 rounded-lg border border-acid/30 bg-acid/[0.07] px-3 py-1.5 text-acid">
                <span className="h-1.5 w-1.5 rounded-full bg-acid" /> Value · stack
              </span>
              <MoveRight className="h-4 w-4 text-white/25" />
              <span className="flex items-center gap-2 rounded-lg border border-lav/30 bg-lav/[0.07] px-3 py-1.5 text-lav">
                <span className="h-1.5 w-1.5 rounded-full bg-lav" /> Managed Heap · ARC
              </span>
              <MoveRight className="h-4 w-4 text-white/25" />
              <span className="flex items-center gap-2 rounded-lg border border-aqua/30 bg-aqua/[0.07] px-3 py-1.5 text-aqua">
                <span className="h-1.5 w-1.5 rounded-full bg-aqua" /> Arena · bulk
              </span>
              <span className="pl-2 text-[11px] text-white/35">
                monotonic — a binding never demotes
              </span>
            </div>
          </div>
        </Reveal>

        {/* rules + code */}
        <div className="mt-14 grid gap-6 lg:grid-cols-[0.9fr_1.1fr]">
          <Reveal delay={0.05}>
            <div className="flex h-full flex-col gap-2.5">
              {RULES.map((r, i) => (
                <button
                  key={r.id}
                  onClick={() => setActive(i)}
                  className={`group rounded-xl border p-4 text-left transition-all duration-300 ${
                    i === active
                      ? "border-acid/45 bg-acid/[0.06]"
                      : "border-white/[0.08] bg-panel hover:border-white/20"
                  }`}
                >
                  <div className="flex items-center gap-3">
                    <span
                      className={`font-mono text-[11px] ${i === active ? "text-acid" : "text-white/30"}`}
                    >
                      R{r.id}
                    </span>
                    <span
                      className={`font-display text-[15px] font-semibold tracking-tight ${
                        i === active ? "text-white" : "text-white/70"
                      }`}
                    >
                      {r.title}
                    </span>
                    <span
                      className={`ml-auto font-mono text-[10px] uppercase tracking-wider transition-opacity ${
                        i === active ? "text-acid opacity-100" : "opacity-0"
                      }`}
                    >
                      analyzing
                    </span>
                  </div>
                  <p className="mt-1.5 pl-9 text-[13px] leading-snug text-white/40">{r.short}</p>
                </button>
              ))}

              <div className="rounded-xl border border-dashed border-white/[0.12] bg-transparent p-4 opacity-60">
                <div className="flex items-center gap-3">
                  <span className="font-mono text-[11px] text-white/30">R6</span>
                  <span className="font-display text-[15px] font-semibold tracking-tight text-white/50">
                    Required Aliasing
                  </span>
                  <Lock className="ml-auto h-3.5 w-3.5 text-white/30" />
                </div>
                <p className="mt-1.5 pl-9 text-[13px] text-white/35">
                  Reserved — pending future language features.
                </p>
              </div>
            </div>
          </Reveal>

          <Reveal delay={0.15}>
            <div className="relative h-full">
              <AnimatePresence mode="wait">
                <motion.div
                  key={rule.id}
                  initial={{ opacity: 0, x: 26 }}
                  animate={{ opacity: 1, x: 0 }}
                  exit={{ opacity: 0, x: -18 }}
                  transition={{ duration: 0.45, ease: EASE }}
                  className="relative"
                >
                  <CodeBlock
                    code={rule.code}
                    title={`rule_${rule.id}.lpp`}
                    highlight={rule.highlight}
                    badge="escape analysis"
                  />
                  {/* scan sweep */}
                  <motion.div
                    key={`scan-${rule.id}`}
                    initial={{ top: "8%", opacity: 0 }}
                    animate={{ top: "88%", opacity: [0, 1, 1, 0] }}
                    transition={{ duration: 1.1, ease: "easeInOut" }}
                    className="pointer-events-none absolute inset-x-4 h-px bg-gradient-to-r from-transparent via-acid to-transparent"
                  />
                </motion.div>
              </AnimatePresence>

              <div className="mt-4 flex items-start gap-3 rounded-xl border border-white/[0.08] bg-panel p-4">
                <ScanSearch className="mt-0.5 h-4 w-4 shrink-0 text-acid" />
                <p className="text-[13.5px] leading-relaxed text-white/55">
                  <AnimatePresence mode="wait">
                    <motion.span
                      key={rule.id}
                      initial={{ opacity: 0 }}
                      animate={{ opacity: 1 }}
                      exit={{ opacity: 0 }}
                      transition={{ duration: 0.35 }}
                    >
                      {rule.verdict}
                    </motion.span>
                  </AnimatePresence>
                </p>
              </div>
            </div>
          </Reveal>
        </div>

        {/* memory map */}
        <Reveal delay={0.1} className="mt-8">
          <div className="overflow-hidden rounded-2xl border border-white/[0.08] bg-panel">
            <div className="flex flex-wrap items-center justify-between gap-2 border-b border-white/[0.07] bg-white/[0.02] px-5 py-3.5">
              <span className="font-mono text-[10px] uppercase tracking-[0.25em] text-white/40">
                runtime memory map — resolved at compile time
              </span>
              <span className="font-mono text-[10px] text-white/30">
                rule {rule.id} · <span className="text-acid">{rule.title.toLowerCase()}</span>
              </span>
            </div>
            <div className="grid md:grid-cols-3">
              {(Object.keys(ZONES) as Target[]).map((key, zi) => {
                const z = ZONES[key];
                const isTarget = rule.target === key;
                return (
                  <div
                    key={key}
                    className={`relative border-white/[0.07] p-6 transition-colors duration-500 ${
                      zi < 2 ? "md:border-r" : ""
                    } ${zi > 0 ? "border-t md:border-t-0" : ""} ${isTarget ? z.bg : ""}`}
                  >
                    <div className="flex items-center gap-2.5">
                      <z.icon className={`h-[18px] w-[18px] ${isTarget ? z.text : "text-white/30"}`} />
                      <span
                        className={`font-display text-lg font-semibold tracking-tight ${
                          isTarget ? "text-white" : "text-white/55"
                        }`}
                      >
                        {z.name}
                      </span>
                      <span
                        className={`rounded border px-1.5 py-0.5 font-mono text-[9px] uppercase tracking-wider ${
                          isTarget ? `${z.border} ${z.text}` : "border-white/10 text-white/30"
                        }`}
                      >
                        {z.sub}
                      </span>
                    </div>
                    <p className="mt-1 font-mono text-[10.5px] text-white/30">{z.desc}</p>

                    <div className="mt-5 flex min-h-[92px] flex-wrap content-start items-start gap-2">
                      {key === "stack" &&
                        rule.stayed.map((s, i) => (
                          <motion.span
                            key={`${rule.id}-${s}`}
                            initial={{ opacity: 0, y: 8 }}
                            animate={{ opacity: 1, y: 0 }}
                            transition={{ delay: 0.35 + i * 0.1, duration: 0.5, ease: EASE }}
                            className="flex items-center gap-1.5 rounded-lg border border-acid/25 bg-acid/[0.05] px-2.5 py-1.5 font-mono text-[11px] text-acid/80"
                          >
                            <span className="h-1 w-1 rounded-full bg-acid/60" />
                            {s}
                          </motion.span>
                        ))}
                      {key === "stack" && rule.stayed.length === 0 && (
                        <span className="font-mono text-[10.5px] italic text-white/25">
                          scalar copies stay — nothing to promote
                        </span>
                      )}
                      {isTarget ? (
                        <motion.span
                          key={rule.id}
                          initial={{ opacity: 0, y: -26, scale: 0.6 }}
                          animate={{ opacity: 1, y: 0, scale: 1 }}
                          transition={{ type: "spring", stiffness: 300, damping: 20, delay: 0.55 }}
                          className={`flex items-center gap-2 rounded-lg border ${z.border} ${z.bg} px-3.5 py-2 font-mono text-[12px] font-semibold ${z.text} shadow-[0_0_30px_-6px_currentColor]`}
                        >
                            <span className={`h-1.5 w-1.5 rounded-full ${z.dot} animate-pulse-dot`} />
                          {rule.varName}
                        </motion.span>
                      ) : (
                        key !== "stack" && (
                          <span className="font-mono text-[10.5px] italic text-white/20">idle</span>
                        )
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
            <div className="border-t border-white/[0.07] bg-white/[0.015] px-5 py-3">
              <AnimatePresence mode="wait">
                <motion.p
                  key={rule.id}
                  initial={{ opacity: 0, y: 6 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0 }}
                  transition={{ duration: 0.4, delay: 0.6 }}
                  className="font-mono text-[11px] text-white/45"
                >
                  <span className="text-white/30">storage verdict → </span>
                  <span className={ZONES[rule.target].text}>
                    {rule.varName} : {ZONES[rule.target].name}
                  </span>
                  <span className="text-white/30"> · inserted automatically · zero annotations written</span>
                </motion.p>
              </AnimatePresence>
            </div>
          </div>
        </Reveal>
      </div>
    </section>
  );
}
