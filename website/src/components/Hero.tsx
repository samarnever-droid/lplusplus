import { useEffect, useRef, useState } from "react";
import { motion, useScroll, useTransform } from "framer-motion";
import { ArrowDown, Cpu, Layers, Network, Boxes, Terminal } from "lucide-react";
import { Code } from "../lib/highlight";
import { EASE } from "../lib/ui";

const SNIPPETS = [
  `def fib(n: Int) -> Int:
    if n < 2:
        return n
    return fib(n - 1) + fib(n - 2)

def main():
    result := fib(35)   # verified through lpp-link
    print(result)`,
  `struct Item:
    value: Int

def create_item() -> Item:
    item := Item()
    return item   # escapes -> Managed Heap
                  # no Box, no Rc, no &`,
  `def identity(value: Item) -> Item:
    return value       # borrowed → retained → ReturnOwned

def main():
    original := Item()
    returned := identity(original)`,
];

function useTypewriter() {
  const [idx, setIdx] = useState(0);
  const [len, setLen] = useState(0);
  useEffect(() => {
    const full = SNIPPETS[idx];
    if (len < full.length) {
      const ch = full[len];
      const delay = ch === "\n" ? 90 : 14 + Math.random() * 26;
      const t = setTimeout(() => setLen((l) => l + 1), delay);
      return () => clearTimeout(t);
    }
    const t = setTimeout(() => {
      setLen(0);
      setIdx((i) => (i + 1) % SNIPPETS.length);
    }, 4600);
    return () => clearTimeout(t);
  }, [len, idx]);
  return SNIPPETS[idx].slice(0, len);
}

const HUD_ROWS = [
  { icon: Layers, name: "result", kind: "Int · scalar", target: "Stack", color: "text-acid", bg: "bg-acid", border: "border-acid/30" },
  { icon: Boxes, name: "item", kind: "struct · escapes", target: "Managed Heap", color: "text-lav", bg: "bg-lav", border: "border-lav/30" },
  { icon: Network, name: "node", kind: "self-referential", target: "Arena", color: "text-aqua", bg: "bg-aqua", border: "border-aqua/30" },
];

export default function Hero() {
  const typed = useTypewriter();
  const ref = useRef<HTMLDivElement>(null);
  const { scrollYProgress } = useScroll({ target: ref, offset: ["start start", "end start"] });
  const codeY = useTransform(scrollYProgress, [0, 1], [0, 110]);
  const hudY = useTransform(scrollYProgress, [0, 1], [0, 60]);
  const fade = useTransform(scrollYProgress, [0, 0.7], [1, 0]);

  return (
    <section ref={ref} id="top" className="relative overflow-hidden pt-[72px]">
      {/* glows */}
      <div className="pointer-events-none absolute -top-40 left-1/2 h-[560px] w-[900px] -translate-x-1/2 rounded-full bg-acid/[0.07] blur-[130px]" />
      <div className="pointer-events-none absolute -right-40 top-1/3 h-[420px] w-[420px] rounded-full bg-lav/[0.06] blur-[120px]" />
      <div className="absolute inset-0 bg-grid [mask-image:radial-gradient(ellipse_75%_65%_at_50%_0%,black,transparent)]" />

      <motion.div style={{ opacity: fade }} className="relative mx-auto max-w-7xl px-5 pb-16 pt-16 md:px-8 md:pt-24 lg:pb-24">
        <div className="grid items-center gap-14 lg:grid-cols-[1.05fr_0.95fr]">
          {/* left — headline */}
          <div>
            <motion.div
              initial={{ opacity: 0, y: 20 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ duration: 0.8, ease: EASE, delay: 0.25 }}
              className="mb-7 inline-flex items-center gap-2.5 rounded-full border border-white/10 bg-white/[0.03] py-1.5 pl-2 pr-4"
            >
              <span className="flex items-center gap-1.5 rounded-full bg-acid/15 px-2.5 py-0.5 font-mono text-[10px] font-semibold uppercase tracking-wider text-acid">
                <span className="h-1.5 w-1.5 rounded-full bg-acid animate-pulse-dot" />
                v0.1 prototype
              </span>
              <span className="font-mono text-[11px] text-white/50">Cranelift AOT backend is live</span>
            </motion.div>

            <h1 className="font-display text-[clamp(2.9rem,7.2vw,5.6rem)] font-bold leading-[0.98] tracking-[-0.03em] text-white">
              {["Readable by default.", "Native when verified.", "Ownership-aware by design."].map((line, i) => (
                <span key={i} className="block overflow-hidden">
                  <motion.span
                    initial={{ y: "110%" }}
                    animate={{ y: 0 }}
                    transition={{ duration: 1, ease: EASE, delay: 0.35 + i * 0.13 }}
                    className={`block ${i === 1 ? "text-acid" : ""} ${i === 2 ? "text-outline" : ""}`}
                  >
                    {line}
                  </motion.span>
                </span>
              ))}
            </h1>

            <motion.p
              initial={{ opacity: 0, y: 20 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ duration: 0.9, ease: EASE, delay: 0.8 }}
              className="mt-7 max-w-xl text-lg leading-relaxed text-white/55"
            >
              L++ is an experimental native language with an{" "}
              <span className="text-white">ownership-aware compiler pipeline</span> — escape
              analysis and MIR decide when values borrow, move, retain, release, or return ownership.{" "}
              Unsupported lifetime cases are rejected instead of silently compiled.
            </motion.p>

            <motion.div
              initial={{ opacity: 0, y: 20 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ duration: 0.9, ease: EASE, delay: 0.95 }}
              className="mt-9 flex flex-wrap items-center gap-4"
            >
              <a
                href="#install"
                className="group flex items-center gap-2.5 rounded-xl bg-acid px-6 py-3.5 font-mono text-sm font-semibold text-ink transition-all duration-300 hover:brightness-110 glow-acid"
              >
                <Terminal className="h-4 w-4" />
                Install the compiler
              </a>
              <a
                href="#memory"
                className="group flex items-center gap-2.5 rounded-xl border border-white/12 bg-white/[0.03] px-6 py-3.5 font-mono text-sm text-white/80 transition-all duration-300 hover:border-acid/40 hover:text-acid"
              >
                <Cpu className="h-4 w-4" />
                See the memory magic
              </a>
            </motion.div>

            <motion.div
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              transition={{ duration: 1, delay: 1.2 }}
              className="mt-10 flex flex-wrap gap-x-8 gap-y-3 font-mono text-[11px] uppercase tracking-[0.18em] text-white/35"
            >
              <span>King20 20 / 20 direct</span>
              <span>~1.6 ms direct link</span>
              <span>ARC + closures + lists</span>
              <span>Linux ELF · Windows COFF W1</span>
            </motion.div>
          </div>

          {/* right — code window + HUD */}
          <motion.div
            initial={{ opacity: 0, y: 40, rotate: 1.5 }}
            animate={{ opacity: 1, y: 0, rotate: 0 }}
            transition={{ duration: 1.1, ease: EASE, delay: 0.55 }}
            className="relative"
          >
            <motion.div style={{ y: codeY }} className="relative">
              <div className="overflow-hidden rounded-2xl border border-white/10 bg-panel/90 shadow-[0_40px_100px_-20px_rgb(0_0_0/0.9)] backdrop-blur">
                <div className="flex items-center gap-2 border-b border-white/[0.07] bg-white/[0.025] px-4 py-3">
                  <span className="h-2.5 w-2.5 rounded-full bg-[#ff5f57]" />
                  <span className="h-2.5 w-2.5 rounded-full bg-[#febc2e]" />
                  <span className="h-2.5 w-2.5 rounded-full bg-[#28c840]" />
                  <span className="ml-3 font-mono text-[11px] text-white/40">hello.lpp</span>
                  <span className="ml-auto flex items-center gap-1.5 font-mono text-[10px] text-acid/80">
                    <span className="h-1.5 w-1.5 rounded-full bg-acid animate-pulse-dot" />
                    lpp --watch
                  </span>
                </div>
                <pre className="code-scroll min-h-[300px] overflow-x-auto p-5 font-mono text-[12.5px] leading-[1.8] md:min-h-[320px] md:text-[13px]">
                  <code className="whitespace-pre">
                    <Code src={typed} />
                    <span className="ml-0.5 inline-block h-[15px] w-[8px] translate-y-[3px] bg-acid animate-blink" />
                  </code>
                </pre>
              </div>

              {/* floating HUD */}
              <motion.div
                style={{ y: hudY }}
                className="relative z-10 mx-4 -mt-8 rounded-2xl border border-white/10 bg-panel2/95 p-4 shadow-[0_30px_70px_-15px_rgb(0_0_0/0.9)] backdrop-blur-xl md:mx-0 md:-ml-10 md:mr-6"
              >
                <div className="mb-3 flex items-center justify-between">
                  <span className="font-mono text-[10px] uppercase tracking-[0.22em] text-white/40">
                    escape analysis · side table
                  </span>
                  <span className="font-mono text-[10px] text-acid">0.4 ms</span>
                </div>
                <div className="space-y-2">
                  {HUD_ROWS.map((r, i) => (
                    <motion.div
                      key={r.name}
                      initial={{ opacity: 0, x: 24 }}
                      animate={{ opacity: 1, x: 0 }}
                      transition={{ duration: 0.7, ease: EASE, delay: 1.15 + i * 0.18 }}
                      className={`flex items-center gap-3 rounded-xl border ${r.border} bg-white/[0.02] px-3 py-2.5`}
                    >
                      <r.icon className={`h-4 w-4 shrink-0 ${r.color}`} />
                      <span className="font-mono text-[12px] text-white/85">{r.name}</span>
                      <span className="hidden font-mono text-[10px] text-white/35 sm:block">{r.kind}</span>
                      <span className="ml-auto flex items-center gap-1.5 font-mono text-[10px] font-semibold">
                        <span className={`h-1.5 w-1.5 rounded-full ${r.bg}`} />
                        <span className={r.color}>{r.target}</span>
                      </span>
                    </motion.div>
                  ))}
                </div>
              </motion.div>
            </motion.div>
          </motion.div>
        </div>

        <motion.a
          href="#language"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          transition={{ delay: 1.8, duration: 1 }}
          className="mt-16 hidden w-max items-center gap-2 font-mono text-[11px] uppercase tracking-[0.25em] text-white/30 transition-colors hover:text-acid lg:flex"
        >
          <ArrowDown className="h-3.5 w-3.5 animate-bounce" />
          scroll to explore
        </motion.a>
      </motion.div>
    </section>
  );
}
