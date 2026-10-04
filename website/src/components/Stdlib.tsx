import { Terminal, Keyboard, FolderOpen, FileJson, Cpu, ListOrdered, Type, Globe } from "lucide-react";
import { SectionHead, Reveal } from "../lib/ui";

const CATS = [
  { icon: Terminal, name: "Console", status: "full", fns: ["print(...)", "print_str(...)"] },
  { icon: Keyboard, name: "Input", status: "full", fns: ["input()"] },
  { icon: FolderOpen, name: "Files", status: "full", fns: ["read_file(p)", "write_file(p, d)"] },
  { icon: FileJson, name: "JSON", status: "full", fns: ["json_parse", "json_get_int", "json_get_str", "json_get_obj", "json_free"] },
  { icon: Cpu, name: "Threads", status: "native", fns: ["spawn fn() -> Void:"] },
  { icon: ListOrdered, name: "Lists", status: "basic+", fns: ["[1, 2, 3]", "list_new", "list_push", "list_get", "list_len", "list_free"] },
  { icon: Type, name: "Strings", status: "basic", fns: ["concat +", "interpolation soon"] },
  { icon: Globe, name: "Networking", status: "soon", fns: ["sockets", "http"] },
];

const STATUS_STYLE: Record<string, string> = {
  full: "border-acid/30 bg-acid/10 text-acid",
  native: "border-lav/30 bg-lav/10 text-lav",
  "basic+": "border-aqua/30 bg-aqua/10 text-aqua",
  basic: "border-ember/30 bg-ember/10 text-ember",
  soon: "border-white/15 bg-white/[0.04] text-white/40",
};

export default function Stdlib() {
  return (
    <section id="stdlib" className="relative border-t border-white/[0.06] py-28 md:py-36">
      <div className="relative mx-auto max-w-7xl px-5 md:px-8">
        <SectionHead
          index="05"
          kicker="Batteries, included and lean"
          title={
            <>
              A stdlib that maps <span className="text-acid">straight to C.</span>
            </>
          }
          desc="Built-ins aren't a runtime — they're thin bindings to optimal C stdlib calls. Full JSON parsing, dynamic lists, native threads and file I/O ship in the compiler today."
        />

        <div className="mt-14 grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          {CATS.map((c, i) => (
            <Reveal key={c.name} delay={(i % 4) * 0.07}>
              <div
                className={`group h-full rounded-2xl border border-white/[0.08] bg-panel p-5 transition-all duration-300 hover:-translate-y-1 hover:border-white/20 ${
                  c.status === "soon" ? "opacity-55" : ""
                }`}
              >
                <div className="flex items-center justify-between">
                  <c.icon className="h-[18px] w-[18px] text-white/60 transition-colors group-hover:text-acid" />
                  <span
                    className={`rounded border px-1.5 py-0.5 font-mono text-[9px] font-semibold uppercase tracking-wider ${STATUS_STYLE[c.status]}`}
                  >
                    {c.status}
                  </span>
                </div>
                <h3 className="mt-4 font-display text-lg font-semibold tracking-tight text-white">
                  {c.name}
                </h3>
                <div className="mt-3 flex flex-wrap gap-1.5">
                  {c.fns.map((f) => (
                    <code
                      key={f}
                      className="rounded-md border border-white/[0.07] bg-ink/70 px-2 py-1 font-mono text-[10.5px] text-white/55"
                    >
                      {f}
                    </code>
                  ))}
                </div>
              </div>
            </Reveal>
          ))}
        </div>

        <Reveal delay={0.15} className="mt-8">
          <div className="flex flex-wrap items-center gap-x-8 gap-y-3 rounded-2xl border border-white/[0.08] bg-panel px-6 py-5">
            <span className="font-mono text-[10px] uppercase tracking-[0.25em] text-white/35">
              data types today
            </span>
            {["Int · 64-bit", "String", "Bool", "Void", "struct", "List[T]"].map((t) => (
              <span key={t} className="font-mono text-[12px] text-white/60">
                <span className="mr-2 text-acid">◆</span>
                {t}
              </span>
            ))}
          </div>
        </Reveal>
      </div>
    </section>
  );
}
