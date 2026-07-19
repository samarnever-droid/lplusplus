import { useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Variable, Code2, Box, Sigma, Cpu, FileJson } from "lucide-react";
import { SectionHead, Reveal, EASE } from "../lib/ui";
import { CodeBlock } from "../lib/highlight";

const TABS = [
  {
    id: "basics",
    label: "Variables",
    icon: Variable,
    title: "variables.lpp",
    code: `def greet() -> Void:
    prefix := "Hello"
    prefix := "Goodbye"  # shadows - never mutates

    mut count := 0
    count = count + 1    # ok: declared mut
    # prefix = "Hi"      <- compile error: immutable

    print(prefix)`,
    note: "Immutable by default. := creates a brand-new binding that safely shadows; = mutates only what you explicitly marked mut.",
  },
  {
    id: "functions",
    label: "Functions",
    icon: Code2,
    title: "functions.lpp",
    code: `def add(a: Int, b: Int) -> Int:
    return a + b

def main() -> Void:
    total := add(20, 22)   # inferred: Int
    print(total)           # 42`,
    note: "Explicit types at the boundaries keep interfaces self-documenting and let the compiler type-check without analyzing the whole call graph. Locals are inferred.",
  },
  {
    id: "structs",
    label: "Structs",
    icon: Box,
    title: "structs.lpp",
    code: `struct Node:
    value: Int
    next: Node   # self-reference -> Arena, automatic

struct Box:
    inner: Node
    count: Int

def safe_return() -> Int:
    my_box := Box()
    return my_box.count   # scalar copy - zero heap traffic`,
    note: "No Box, no &, no allocation modifiers. Value semantics on the stack where possible; self-referential structs promote to Arenas on their own.",
  },
  {
    id: "closures",
    label: "Closures",
    icon: Sigma,
    title: "closures.lpp",
    code: `def process() -> Void:
    map(items, fn(x) -> x * 2)      # inline

    multiplier := 5
    callback := fn(x):              # block closure
        mut y := x * multiplier     # captured + cloned
        return y + 1

    print(callback(21))             # 106`,
    note: "fn closures infer parameter and return types. Immutable scalars are cloned by value; escaping captures promote to the Managed Heap automatically.",
  },
  {
    id: "concurrency",
    label: "Concurrency",
    icon: Cpu,
    title: "threads.lpp",
    code: `def parallel_work() -> Void:
    shared_readonly := 100
    mut shared_state := 0

    spawn fn() -> Void:
        # readonly: copied by value
        # shared_state: mut -> Managed Heap
        print(shared_readonly, shared_state)`,
    note: "spawn launches native threads. The compiler sees what crosses the boundary and picks copy vs. shared heap storage — thread safety without ceremony.",
  },
  {
    id: "stdlib",
    label: "JSON & Files",
    icon: FileJson,
    title: "config.lpp",
    code: `def load_config() -> Void:
    raw := read_file("config.json")
    root := json_parse(raw)

    name := json_get_str(root, "name")
    retries := json_get_int(root, "retries")
    print(name, retries)

    json_free(root)   # recursive cleanup`,
    note: "Built-ins map directly to optimal C stdlib calls: console, files, dynamic lists, and a full JSON parser — no package install required.",
  },
];

export default function Showcase() {
  const [active, setActive] = useState(0);
  const tab = TABS[active];

  return (
    <section id="syntax" className="relative border-t border-white/[0.06] py-28 md:py-36">
      <div className="pointer-events-none absolute right-0 top-24 h-[380px] w-[380px] rounded-full bg-acid/[0.05] blur-[120px]" />
      <div className="relative mx-auto max-w-7xl px-5 md:px-8">
        <SectionHead
          index="03"
          kicker="The language"
          title={
            <>
              Python's handwriting, <span className="text-acid">a systems soul.</span>
            </>
          }
          desc="Significant whitespace and colons define blocks — no curly braces, no semicolons. Every construct is designed to disappear so the logic can speak."
        />

        <Reveal delay={0.1} className="mt-12">
          <div className="flex flex-wrap gap-2">
            {TABS.map((t, i) => (
              <button
                key={t.id}
                onClick={() => setActive(i)}
                className={`flex items-center gap-2 rounded-lg border px-4 py-2.5 font-mono text-[12px] transition-all duration-300 ${
                  i === active
                    ? "border-acid/50 bg-acid/10 text-acid"
                    : "border-white/[0.09] bg-panel text-white/50 hover:border-white/25 hover:text-white/80"
                }`}
              >
                <t.icon className="h-3.5 w-3.5" />
                {t.label}
              </button>
            ))}
          </div>
        </Reveal>

        <div className="mt-6 grid gap-6 lg:grid-cols-[1.25fr_0.75fr]">
          <Reveal delay={0.15}>
            <AnimatePresence mode="wait">
              <motion.div
                key={tab.id}
                initial={{ opacity: 0, y: 16 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: -10 }}
                transition={{ duration: 0.4, ease: EASE }}
              >
                <CodeBlock code={tab.code} title={tab.title} badge="l++" />
              </motion.div>
            </AnimatePresence>
          </Reveal>
          <Reveal delay={0.22}>
            <div className="flex h-full flex-col justify-between gap-6 rounded-2xl border border-white/[0.08] bg-panel p-7">
              <div>
                <span className="font-mono text-[10px] uppercase tracking-[0.25em] text-white/35">
                  why it works this way
                </span>
                <AnimatePresence mode="wait">
                  <motion.p
                    key={tab.id}
                    initial={{ opacity: 0, y: 10 }}
                    animate={{ opacity: 1, y: 0 }}
                    exit={{ opacity: 0 }}
                    transition={{ duration: 0.4, delay: 0.1 }}
                    className="mt-4 text-[15.5px] leading-relaxed text-white/60"
                  >
                    {tab.note}
                  </motion.p>
                </AnimatePresence>
              </div>
              <div className="rounded-xl border border-white/[0.07] bg-ink/60 p-4 font-mono text-[11.5px] leading-relaxed text-white/40">
                <span className="text-acid">design note —</span> the parser already supports{" "}
                <span className="text-white/70">if / else</span>,{" "}
                <span className="text-white/70">while</span>, relational operators and module
                merging via <span className="text-white/70">import</span>. for-loops land with
                iterator protocols next.
              </div>
            </div>
          </Reveal>
        </div>
      </div>
    </section>
  );
}
