import { ArrowUpRight } from "lucide-react";

const COLS = [
  {
    title: "Language",
    links: [
      { label: "Syntax basics", href: "#syntax" },
      { label: "Memory model", href: "#memory" },
      { label: "Escape rules", href: "#memory" },
      { label: "Standard library", href: "#stdlib" },
    ],
  },
  {
    title: "Compiler",
    links: [
      { label: "Performance", href: "#performance" },
      { label: "Cranelift AOT", href: "#performance" },
      { label: "C transpiler", href: "#performance" },
      { label: "Roadmap", href: "#roadmap" },
    ],
  },
  {
    title: "Start",
    links: [
      { label: "Install", href: "#install" },
      { label: "lpp -h", href: "#install" },
      { label: "Back to top", href: "#top" },
    ],
  },
];

export default function Footer() {
  return (
    <footer className="relative overflow-hidden border-t border-white/[0.07]">
      <div className="relative mx-auto max-w-7xl px-5 pb-10 pt-20 md:px-8">
        <div className="grid gap-12 md:grid-cols-[1.2fr_1.8fr]">
          <div>
            <a href="#top" className="flex w-max items-center gap-3">
              <span className="grid h-10 w-10 place-items-center rounded-lg bg-acid font-mono text-base font-bold text-ink">
                L++
              </span>
              <span className="font-mono text-[11px] leading-tight text-white/40">
                the hybrid memory
                <br />
                language
              </span>
            </a>
            <p className="mt-6 max-w-sm text-[14.5px] leading-relaxed text-white/40">
              An experimental prototype proving that memory management can be a compiler's job —
              not a programmer's burden, not a runtime's tax.
            </p>
            <a
              href="#install"
              className="group mt-7 inline-flex items-center gap-2 rounded-xl border border-acid/30 bg-acid/[0.06] px-5 py-3 font-mono text-[12.5px] text-acid transition-all hover:bg-acid hover:text-ink"
            >
              Build something fast
              <ArrowUpRight className="h-4 w-4 transition-transform group-hover:translate-x-0.5 group-hover:-translate-y-0.5" />
            </a>
          </div>

          <div className="grid grid-cols-2 gap-8 sm:grid-cols-3">
            {COLS.map((c) => (
              <div key={c.title}>
                <p className="font-mono text-[10px] uppercase tracking-[0.25em] text-white/30">
                  {c.title}
                </p>
                <ul className="mt-4 space-y-2.5">
                  {c.links.map((l) => (
                    <li key={l.label}>
                      <a
                        href={l.href}
                        className="text-[13.5px] text-white/50 transition-colors hover:text-acid"
                      >
                        {l.label}
                      </a>
                    </li>
                  ))}
                </ul>
              </div>
            ))}
          </div>
        </div>

        <div className="mt-16 flex flex-col items-start justify-between gap-4 border-t border-white/[0.07] pt-7 font-mono text-[11px] text-white/30 sm:flex-row sm:items-center">
          <span>© 2026 the L++ project — experimental prototype</span>
          <span>
            no garbage collectors were harmed <span className="text-acid">++</span> no borrow
            checkers either
          </span>
        </div>
      </div>

      {/* giant watermark */}
      <div
        aria-hidden
        className="pointer-events-none select-none text-center font-mono text-[26vw] font-bold leading-[0.72] tracking-tighter text-white/[0.025]"
      >
        L++
      </div>
    </footer>
  );
}
