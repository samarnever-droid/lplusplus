import { Feather, Zap, ShieldCheck } from "lucide-react";
import { SectionHead, Reveal } from "../lib/ui";
import { Code } from "../lib/highlight";

const PILLARS = [
  {
    icon: Feather,
    color: "text-acid",
    ring: "group-hover:border-acid/40",
    chip: "border-acid/25 bg-acid/10 text-acid",
    title: "Readable as Python",
    body: "Significant whitespace, colon blocks, and := declarations. Types are explicit at function boundaries and inferred everywhere else. Code that reads like prose.",
    code: `def greet(name: String) -> Void:\n    msg := "Hello, " + name\n    print(msg)`,
    tag: "syntax.lpp",
  },
  {
    icon: Zap,
    color: "text-ember",
    ring: "group-hover:border-ember/40",
    chip: "border-ember/25 bg-ember/10 text-ember",
    title: "Fast as C",
    body: "The Cranelift AOT backend emits native x86-64 machine code — no VM, no interpreter, no JIT warmup. Recursive fib(35) runs in ~64 ms, toe-to-toe with gcc -O2.",
    code: `# source -> native .exe in ~3 ms\n$ lpp main.lpp\n$ .\\main.exe   # 138 KB, zero deps`,
    tag: "shell",
  },
  {
    icon: ShieldCheck,
    color: "text-lav",
    ring: "group-hover:border-lav/40",
    chip: "border-lav/25 bg-lav/10 text-lav",
    title: "Safe as Rust",
    body: "Immutable by default. Memory is managed by the compiler's escape analyzer — stack, ARC heap, or arena — with no data races by design, and no borrow checker to fight.",
    code: `x := 5\n# x = 6   <- compile error\nmut y := 10\ny = 20      # explicit intent`,
    tag: "safety.lpp",
  },
];

export default function Pillars() {
  return (
    <section id="language" className="relative mx-auto max-w-7xl px-5 py-28 md:px-8 md:py-36">
      <SectionHead
        index="01"
        kicker="One language, three promises"
        title={
          <>
            The triangle, <span className="text-acid">finally closed.</span>
          </>
        }
        desc="Languages have always forced a trade: pick two of readability, speed, and safety. L++ closes the triangle by moving memory management into the compiler's semantic analysis — where it belongs."
      />

      <div className="mt-14 grid gap-5 md:grid-cols-3">
        {PILLARS.map((p, i) => (
          <Reveal key={p.title} delay={i * 0.12}>
            <div
              className={`group relative flex h-full flex-col overflow-hidden rounded-2xl border border-white/[0.08] bg-panel p-7 transition-all duration-500 hover:-translate-y-1.5 ${p.ring}`}
            >
              <div className="pointer-events-none absolute -right-16 -top-16 h-40 w-40 rounded-full bg-white/[0.03] blur-2xl transition-opacity duration-500 group-hover:opacity-100" />
              <p.icon className={`h-6 w-6 ${p.color}`} />
              <h3 className="mt-5 font-display text-2xl font-semibold tracking-tight text-white">
                {p.title}
              </h3>
              <p className="mt-3 flex-1 text-[15px] leading-relaxed text-white/50">{p.body}</p>
              <div className="mt-6 overflow-hidden rounded-xl border border-white/[0.07] bg-ink/80">
                <div className="flex items-center justify-between border-b border-white/[0.06] px-3.5 py-2">
                  <span className="font-mono text-[10px] text-white/35">{p.tag}</span>
                  <span className={`rounded border px-1.5 py-0.5 font-mono text-[9px] font-semibold uppercase tracking-wider ${p.chip}`}>
                    l++
                  </span>
                </div>
                <pre className="code-scroll overflow-x-auto p-4 font-mono text-[12px] leading-[1.7]">
                  <code className="whitespace-pre">
                    <Code src={p.code} />
                  </code>
                </pre>
              </div>
            </div>
          </Reveal>
        ))}
      </div>
    </section>
  );
}
