import { Check, Loader, Circle, FlaskConical } from "lucide-react";
import { SectionHead, Reveal } from "../lib/ui";

const LANES = [
  {
    title: "Verified now",
    icon: Check,
    color: "text-acid",
    chip: "border-acid/30 bg-acid/10 text-acid",
    items: [
      { name: "King20 direct ELF", note: "20 / 20 Stable workloads without a host final linker" },
      { name: "Ownership-aware MIR", note: "borrow, move, retain, release, ReturnOwned" },
      { name: "ARC runtime", note: "structs, closures, List[Int], List[Custom], destructors" },
      { name: "Linux native toolchain", note: "Cranelift ELF + lpp-link + syscall runtime" },
    ],
  },
  {
    title: "In motion",
    icon: Loader,
    color: "text-lav",
    chip: "border-lav/30 bg-lav/10 text-lav",
    items: [
      { name: "Windows W1", note: "COFF inspection and MSVC fallback in CI" },
      { name: "Windows PE linker", note: "section merge and AMD64 relocations next" },
      { name: "Scalability", note: "escape analysis and AOT lowering are current 100k LOC targets" },
    ],
  },
  {
    title: "Next boundaries",
    icon: Circle,
    color: "text-white/50",
    chip: "border-white/15 bg-white/[0.04] text-white/45",
    items: [
      { name: "Writable direct ELF data", note: ".data, .bss, writable relocations" },
      { name: "Runtime expansion", note: "files, networking, threads, JSON" },
      { name: "Windows 20 / 20", note: "King20 through direct PE output" },
      { name: "Upstream Linguist", note: "awaiting maintainer review" },
    ],
  },
];

export default function Roadmap() {
  return (
    <section id="roadmap" className="relative border-t border-white/[0.06] py-28 md:py-36">
      <div className="pointer-events-none absolute right-0 top-0 h-[360px] w-[360px] rounded-full bg-lav/[0.05] blur-[120px]" />
      <div className="relative mx-auto max-w-7xl px-5 md:px-8">
        <SectionHead
          index="06"
          kicker="Honest about the prototype"
          title={
            <>
              Working today. <span className="text-acid">Ambitious tomorrow.</span>
            </>
          }
          desc="L++ is an active prototype: the compiled core is real and fast, and the surface area grows every week. Here's exactly where things stand."
        />

        <div className="mt-14 grid gap-5 lg:grid-cols-3">
          {LANES.map((lane, li) => (
            <Reveal key={lane.title} delay={li * 0.1}>
              <div className="h-full rounded-2xl border border-white/[0.08] bg-panel p-6">
                <div className="mb-6 flex items-center gap-2.5">
                  <lane.icon className={`h-4 w-4 ${lane.color}`} />
                  <span className="font-display text-lg font-semibold tracking-tight text-white">
                    {lane.title}
                  </span>
                  <span className={`ml-auto rounded border px-1.5 py-0.5 font-mono text-[9px] font-semibold uppercase tracking-wider ${lane.chip}`}>
                    {lane.items.length}
                  </span>
                </div>
                <div className="space-y-3">
                  {lane.items.map((it) => (
                    <div
                      key={it.name}
                      className="rounded-xl border border-white/[0.06] bg-ink/50 px-4 py-3 transition-colors hover:border-white/15"
                    >
                      <p className="font-mono text-[12.5px] font-medium text-white/80">{it.name}</p>
                      <p className="mt-1 text-[12px] text-white/35">{it.note}</p>
                    </div>
                  ))}
                </div>
              </div>
            </Reveal>
          ))}
        </div>

        <Reveal delay={0.2} className="mt-6">
          <div className="flex items-start gap-4 rounded-2xl border border-dashed border-aqua/25 bg-aqua/[0.03] p-6">
            <FlaskConical className="mt-0.5 h-5 w-5 shrink-0 text-aqua" />
            <div>
              <p className="font-mono text-[13px] font-semibold text-aqua">
                Research track — Escape Rule 6: Required Aliasing
              </p>
              <p className="mt-1.5 max-w-3xl text-[14px] leading-relaxed text-white/45">
                The sixth promotion rule depends on language features still being designed. When it
                lands, the hybrid model will cover the last known escape pattern — keeping the
                "no GC, no borrow checker" promise complete.
              </p>
            </div>
          </div>
        </Reveal>
      </div>
    </section>
  );
}
