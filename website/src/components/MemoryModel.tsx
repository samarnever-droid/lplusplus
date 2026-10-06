import { useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Layers, Boxes, Network, Lock, MoveRight, ScanSearch } from "lucide-react";
import { SectionHead, Reveal, EASE } from "../lib/ui";
import { CodeBlock } from "../lib/highlight";

type Target = "frame" | "owned" | "shared";

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
    title: "Non-escaping Local",
    short: "A value used only inside one function can remain frame-placed.",
    code: `def calculate() -> Int:\n    base := 40\n    bonus := 2\n    result := base + bonus\n    return result`,
    highlight: [2, 3, 4],
    target: "frame",
    varName: "result",
    stayed: ["base", "bonus"],
    verdict:
      "The values never require managed identity outside the function, so the ownership plan keeps them in the function frame.",
  },
  {
    id: 2,
    title: "Returned Aggregate",
    short: "An aggregate returned to its caller outlives the callee frame.",
    code: `struct Item:\n    value: Int\n\ndef create_item() -> Item:\n    item := Item(42)\n    return item`,
    highlight: [5, 6],
    target: "owned",
    varName: "item",
    stayed: [],
    verdict:
      "item escapes its defining frame but has no ownership cycle. The planner selects Owned and recursive deallocation remains sound.",
  },
  {
    id: 3,
    title: "Owned Container",
    short: "A list can own nested acyclic values without making their type cyclic.",
    code: `struct Item:\n    value: Int\n\ndef build() -> List[Item]:\n    item := Item(7)\n    values := [item]\n    return values`,
    highlight: [5, 6, 7],
    target: "owned",
    varName: "values",
    stayed: [],
    verdict:
      "The containment graph records List[Item] → Item. Because the graph is acyclic, the escaping container remains recursively Owned.",
  },
  {
    id: 4,
    title: "Async Task Value",
    short: "Tasks participate in the same typed containment and escape plan.",
    code: `async def load_message() -> Str:\n    return "ready"\n\nasync def main():\n    message := load_message().await\n    print_str(message)`,
    highlight: [1, 5],
    target: "owned",
    varName: "message",
    stayed: [],
    verdict:
      "Task and result values are planned explicitly. Await retains the result owner while task destruction releases its own managed state.",
  },
  {
    id: 5,
    title: "Ownership Cycle",
    short: "A self-edge or strongly connected component requires shared strategy.",
    code: `struct Node:\n    value: Int\n    next: Node\n\ndef main():\n    node := Node(1)`,
    highlight: [3, 6],
    target: "shared",
    varName: "node",
    stayed: [],
    verdict:
      "The containment graph detects the recursive type cycle. That node strategy becomes Shared, with ARC behavior and a pinned cycle set in the execution proof.",
  },
];

const ZONES: Record<
  Target,
  { icon: typeof Layers; name: string; sub: string; desc: string; text: string; border: string; bg: string; dot: string }
> = {
  frame: {
    icon: Layers,
    name: "Frame",
    sub: "local arena",
    desc: "non-escaping value cells",
    text: "text-acid",
    border: "border-acid/45",
    bg: "bg-acid/[0.06]",
    dot: "bg-acid",
  },
  owned: {
    icon: Boxes,
    name: "Owned",
    sub: "recursive",
    desc: "escaping · acyclic",
    text: "text-lav",
    border: "border-lav/45",
    bg: "bg-lav/[0.06]",
    dot: "bg-lav",
  },
  shared: {
    icon: Network,
    name: "Shared",
    sub: "ARC",
    desc: "cycle-aware sharing",
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
          kicker="Safety engineered into the pipeline"
          title={
            <>
              You write intent. The compiler{" "}
              <span className="text-acid">proves the ownership plan.</span>
            </>
          }
          desc="Validated typed MIR feeds escape analysis, a containment graph, and cycle detection. Each managed value cell is planned as Frame, Owned, or Shared, then checked by an independent plan proof and static ownership-balance analysis."
        />

        {/* ownership strategies */}
        <Reveal delay={0.1} className="mt-10">
          <div className="flex flex-wrap items-center gap-x-4 gap-y-3 rounded-2xl border border-white/[0.08] bg-panel px-5 py-4">
            <span className="font-mono text-[10px] uppercase tracking-[0.25em] text-white/35">
              ownership strategies
            </span>
            <div className="flex flex-wrap items-center gap-x-3 gap-y-2 font-mono text-[12px]">
              <span className="flex items-center gap-2 rounded-lg border border-acid/30 bg-acid/[0.07] px-3 py-1.5 text-acid">
                <span className="h-1.5 w-1.5 rounded-full bg-acid" /> Frame · non-escaping
              </span>
              <MoveRight className="h-4 w-4 text-white/25" />
              <span className="flex items-center gap-2 rounded-lg border border-lav/30 bg-lav/[0.07] px-3 py-1.5 text-lav">
                <span className="h-1.5 w-1.5 rounded-full bg-lav" /> Owned · acyclic
              </span>
              <MoveRight className="h-4 w-4 text-white/25" />
              <span className="flex items-center gap-2 rounded-lg border border-aqua/30 bg-aqua/[0.07] px-3 py-1.5 text-aqua">
                <span className="h-1.5 w-1.5 rounded-full bg-aqua" /> Shared · cycle-aware ARC
              </span>
              <span className="pl-2 text-[11px] text-white/35">
                selected from typed containment and escape facts
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

              <div className="rounded-xl border border-dashed border-white/[0.12] bg-transparent p-4 opacity-70">
                <div className="flex items-center gap-3">
                  <span className="font-mono text-[11px] text-white/30">PROOF</span>
                  <span className="font-display text-[15px] font-semibold tracking-tight text-white/60">
                    Independent Verification
                  </span>
                  <Lock className="ml-auto h-3.5 w-3.5 text-white/30" />
                </div>
                <p className="mt-1.5 pl-9 text-[13px] text-white/40">
                  A separate verifier recomputes placement, graph cycles, arenas, and determinism.
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
                      {key === "frame" &&
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
                      {key === "frame" && rule.stayed.length === 0 && (
                        <span className="font-mono text-[10.5px] italic text-white/25">
                          no additional frame cells shown
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
                        key !== "frame" && (
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
                  <span className="text-white/30"> · planned automatically from typed MIR</span>
                </motion.p>
              </AnimatePresence>
            </div>
          </div>
        </Reveal>
      </div>
    </section>
  );
}
