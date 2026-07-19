import { useEffect, useRef, useState } from "react";
import { animate, motion, useInView } from "framer-motion";
import { Timer, Package, Gauge, GitBranch } from "lucide-react";
import { SectionHead, Reveal, EASE } from "../lib/ui";

function Counter({
  to,
  decimals = 0,
  suffix = "",
  prefix = "",
}: {
  to: number;
  decimals?: number;
  suffix?: string;
  prefix?: string;
}) {
  const ref = useRef<HTMLSpanElement>(null);
  const inView = useInView(ref, { once: true, margin: "-60px" });
  const [val, setVal] = useState(0);
  useEffect(() => {
    if (!inView) return;
    const c = animate(0, to, {
      duration: 1.8,
      ease: EASE,
      onUpdate: (v) => setVal(v),
    });
    return () => c.stop();
  }, [inView, to]);
  return (
    <span ref={ref}>
      {prefix}
      {val.toFixed(decimals)}
      {suffix}
    </span>
  );
}

const BENCH = [
  { name: "Host final link", ms: 200, label: "~200 ms", color: "bg-white/15", text: "text-white/40", note: "fallback path" },
  { name: "L++ direct ELF link", ms: 1.7, label: "~1.7 ms", color: "bg-acid", text: "text-acid", note: "King20 direct", glow: true },
  { name: "100k escape analysis", ms: 691, label: "691 ms", color: "bg-lav", text: "text-lav", note: "scalability target" },
  { name: "100k Cranelift AOT", ms: 634, label: "634 ms", color: "bg-aqua", text: "text-aqua", note: "scalability target" },
];

const STATS = [
  { icon: Timer, value: <Counter to={1.7} decimals={1} suffix=" ms" />, label: "direct ELF link", sub: "King20 Linux x86-64 path" },
  { icon: Gauge, value: <Counter to={20} suffix=" / 20" />, label: "direct linker gate", sub: "King20 Stable verified" },
  { icon: Package, value: <Counter to={100} suffix="k LOC" />, label: "scalability workload", sub: "phase timings recorded" },
  { icon: GitBranch, value: <Counter to={4} />, label: "toolchain paths", sub: "AOT · C fallback · ELF · COFF W1" },
];

export default function Performance() {
  const chartRef = useRef<HTMLDivElement>(null);
  const inView = useInView(chartRef, { once: true, margin: "-100px" });
  const max = Math.sqrt(1280);

  return (
    <section id="performance" className="relative border-t border-white/[0.06] py-28 md:py-36">
      <div className="pointer-events-none absolute -left-32 top-1/3 h-[420px] w-[420px] rounded-full bg-acid/[0.05] blur-[130px]" />
      <div className="relative mx-auto max-w-7xl px-5 md:px-8">
        <SectionHead
          index="04"
          kicker="Measured, not promised"
          title={
            <>
              Native speed, <span className="text-acid">measured in milliseconds.</span>
            </>
          }
          desc="L++ measures every phase: parsing, ownership analysis, MIR, Cranelift, and linking. Direct ELF removes the host final-link bottleneck for the verified Linux subset."
        />

        <div className="mt-14 grid gap-6 lg:grid-cols-[1.05fr_0.95fr]">
          {/* benchmark chart */}
          <Reveal>
            <div ref={chartRef} className="flex h-full flex-col rounded-2xl border border-white/[0.08] bg-panel p-7">
              <div className="mb-8 flex items-center justify-between">
                <span className="font-mono text-[10px] uppercase tracking-[0.25em] text-white/40">
                  recursive fib(35) — wall time
                </span>
                <span className="font-mono text-[10px] text-white/25">√ scale · lower is better</span>
              </div>
              <div className="flex flex-1 flex-col justify-center gap-6">
                {BENCH.map((b, i) => (
                  <div key={b.name}>
                    <div className="mb-2 flex items-baseline justify-between font-mono text-[12px]">
                      <span className={b.glow ? "font-semibold text-white" : "text-white/55"}>
                        {b.name}
                        <span className="ml-2 text-[10px] uppercase tracking-wider text-white/25">{b.note}</span>
                      </span>
                      <span className={`font-semibold ${b.text}`}>{b.label}</span>
                    </div>
                    <div className="h-3 overflow-hidden rounded-full bg-white/[0.05]">
                      <motion.div
                        initial={{ width: 0 }}
                        animate={inView ? { width: `${Math.max((Math.sqrt(b.ms) / max) * 100, 6)}%` } : {}}
                        transition={{ duration: 1.4, delay: 0.15 + i * 0.12, ease: EASE }}
                        className={`h-full rounded-full ${b.color} ${b.glow ? "glow-acid" : ""}`}
                      />
                    </div>
                  </div>
                ))}
              </div>
              <p className="mt-8 border-t border-white/[0.06] pt-4 font-mono text-[11px] leading-relaxed text-white/30">
                L++ matches optimized C within noise while compiling the whole program before
                Python finishes importing.
              </p>
            </div>
          </Reveal>

          {/* pipeline card */}
          <Reveal delay={0.12}>
            <div className="flex h-full flex-col rounded-2xl border border-white/[0.08] bg-panel p-7">
              <span className="font-mono text-[10px] uppercase tracking-[0.25em] text-white/40">
                compilation pipeline
              </span>
              <div className="mt-6 flex-1 space-y-0">
                {[
                  { t: "Parse & semantic analysis", d: "AST + type checking with explicit interface signatures", c: "text-white", dot: "bg-white/60" },
                  { t: "Escape analysis pass", d: "side-table maps every binding → Stack / ARC Heap / Arena", c: "text-acid", dot: "bg-acid" },
                  { t: "MIR + ARC insertion", d: "retain / release calls placed automatically", c: "text-lav", dot: "bg-lav" },
                  { t: "Cranelift codegen", d: "native x86-64 object file, or optimized C via transpiler", c: "text-aqua", dot: "bg-aqua" },
                  { t: "Link with lpp_runtime", d: "MSVC link.exe + lean C runtime → main.exe", c: "text-ember", dot: "bg-ember" },
                ].map((s, i, arr) => (
                  <motion.div
                    key={s.t}
                    initial={{ opacity: 0, x: -18 }}
                    whileInView={{ opacity: 1, x: 0 }}
                    viewport={{ once: true, margin: "-40px" }}
                    transition={{ duration: 0.6, delay: i * 0.1, ease: EASE }}
                    className="relative flex gap-4 pb-6 last:pb-0"
                  >
                    {i < arr.length - 1 && (
                      <span className="absolute left-[5px] top-5 h-full w-px bg-white/[0.09]" />
                    )}
                    <span className={`mt-1.5 h-[11px] w-[11px] shrink-0 rounded-full border-2 border-ink ${s.dot}`} />
                    <div>
                      <p className={`font-mono text-[13px] font-semibold ${s.c}`}>{s.t}</p>
                      <p className="mt-1 text-[13px] leading-snug text-white/40">{s.d}</p>
                    </div>
                  </motion.div>
                ))}
              </div>
            </div>
          </Reveal>
        </div>

        {/* stat cards */}
        <div className="mt-6 grid gap-5 sm:grid-cols-2 lg:grid-cols-4">
          {STATS.map((s, i) => (
            <Reveal key={s.label} delay={i * 0.08}>
              <div className="group rounded-2xl border border-white/[0.08] bg-panel p-6 transition-colors duration-300 hover:border-acid/30">
                <s.icon className="h-[18px] w-[18px] text-acid/70" />
                <p className="mt-4 font-display text-4xl font-bold tracking-tight text-white">
                  {s.value}
                </p>
                <p className="mt-1.5 font-mono text-[11px] uppercase tracking-[0.14em] text-white/60">
                  {s.label}
                </p>
                <p className="mt-2 text-[12.5px] leading-snug text-white/35">{s.sub}</p>
              </div>
            </Reveal>
          ))}
        </div>
      </div>
    </section>
  );
}
