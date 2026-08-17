import { useState } from "react";
import { Package, Terminal, ArrowUpRight, Check, Copy, Sparkles, Box, ShieldCheck, Database, Layers } from "lucide-react";
import { SectionHead, Reveal } from "../lib/ui";

const HIGHLIGHT_PACKAGES = [
  {
    name: "lreact",
    version: "1.2.0",
    desc: "Declarative reactive UI framework and desktop web app runtime with native L++ IPC bridge.",
    downloads: 4890,
    category: "Desktop & UI",
    color: "text-acid border-acid/30 bg-acid/10",
  },
  {
    name: "lpp-graph",
    version: "1.0.0",
    desc: "Weighted directed graph algorithms: Dijkstra shortest-path, Kahn topological sort, and DAG traversal.",
    downloads: 1420,
    category: "Algorithms",
    color: "text-emerald-400 border-emerald-400/30 bg-emerald-400/10",
  },
  {
    name: "lppsqlite",
    version: "1.0.0",
    desc: "Embedded SQLite-compatible database with connection pooling and type-safe prepared queries.",
    downloads: 4120,
    category: "Database",
    color: "text-cyan-400 border-cyan-400/30 bg-cyan-400/10",
  },
  {
    name: "lppdb",
    version: "1.0.0",
    desc: "ACID embedded document database with JSON query indexing and binary page persistence.",
    downloads: 2310,
    category: "Database",
    color: "text-amber-400 border-amber-400/30 bg-amber-400/10",
  },
  {
    name: "lpp-git",
    version: "0.8.0",
    desc: "Pure L++ Git object store parser, commit DAG walker, and tree resolver.",
    downloads: 2100,
    category: "Dev Tools",
    color: "text-lav border-lav/30 bg-lav/10",
  },
  {
    name: "compresslpp",
    version: "1.0.0",
    desc: "Lossless compression library: LZ4, Zstandard, and DEFLATE with streaming ZIP archive support.",
    downloads: 1730,
    category: "Compression",
    color: "text-rose-400 border-rose-400/30 bg-rose-400/10",
  },
  {
    name: "lpp-json",
    version: "1.0.0",
    desc: "SIMD zero-allocation JSON parser with native struct deserialization.",
    downloads: 3560,
    category: "Data Structures",
    color: "text-sky-400 border-sky-400/30 bg-sky-400/10",
  },
  {
    name: "lpp-sha256",
    version: "1.0.0",
    desc: "Cryptographic SHA-256 hash and HMAC verification engine written in pure L++.",
    downloads: 2450,
    category: "Security",
    color: "text-acid border-acid/30 bg-acid/10",
  },
];

export default function PackagesShowcase() {
  const [copiedId, setCopiedId] = useState<string | null>(null);

  const copy = (text: string, id: string) => {
    navigator.clipboard.writeText(text);
    setCopiedId(id);
    setTimeout(() => setCopiedId(null), 2000);
  };

  return (
    <section id="packages" className="relative border-t border-white/[0.06] py-24 md:py-32 bg-[#07090d]">
      <div className="pointer-events-none absolute -right-32 top-1/4 h-[500px] w-[500px] rounded-full bg-acid/[0.04] blur-[150px]" />
      
      <div className="relative mx-auto max-w-7xl px-5 md:px-8 space-y-12">
        <div className="flex flex-col md:flex-row md:items-end justify-between gap-6">
          <SectionHead
            index="05"
            kicker="Official Package Registry"
            title={
              <>
                Native Libraries, <span className="text-acid">Ready to Install.</span>
              </>
            }
            desc="Explore 16 officially verified packages published to registry.lplusplus.bond with 256-bit cryptographically signed tokens and zero backend URL exposure."
          />

          <div className="shrink-0">
            <a
              href="/packages.html"
              className="inline-flex items-center gap-2 rounded-xl bg-acid px-6 py-3 font-mono text-xs font-bold text-ink hover:brightness-110 shadow-[0_0_20px_rgba(200,241,75,0.3)]"
            >
              <Package className="h-4 w-4" />
              Explore All 16 Packages
              <ArrowUpRight className="h-4 w-4" />
            </a>
          </div>
        </div>

        {/* Packages Grid */}
        <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4">
          {HIGHLIGHT_PACKAGES.map((pkg) => (
            <div
              key={pkg.name}
              className="flex flex-col justify-between rounded-2xl border border-white/10 bg-white/[0.02] p-5 space-y-4 transition-all duration-300 hover:border-acid/40 hover:bg-white/[0.04] group"
            >
              <div className="space-y-2.5">
                <div className="flex items-center justify-between">
                  <span className={`rounded-md border px-2 py-0.5 font-mono text-[10px] font-bold uppercase tracking-wider ${pkg.color}`}>
                    {pkg.category}
                  </span>
                  <span className="font-mono text-xs text-white/40">
                    v{pkg.version}
                  </span>
                </div>

                <h4 className="font-mono text-base font-bold text-white group-hover:text-acid transition-colors">
                  {pkg.name}
                </h4>

                <p className="text-xs text-white/60 leading-relaxed line-clamp-3">
                  {pkg.desc}
                </p>
              </div>

              <div className="space-y-3 pt-2 border-t border-white/10">
                <div className="flex items-center justify-between text-[11px] font-mono text-white/40">
                  <span>{pkg.downloads.toLocaleString()} downloads</span>
                  <span className="text-emerald-400 flex items-center gap-1">
                    <ShieldCheck className="h-3 w-3" />
                    Verified
                  </span>
                </div>

                <button
                  onClick={() => copy(`lpp add ${pkg.name}`, pkg.name)}
                  className="flex w-full items-center justify-center gap-1.5 rounded-xl border border-white/10 bg-white/5 py-2 font-mono text-xs text-white/80 hover:border-acid/50 hover:text-acid transition-all"
                >
                  {copiedId === pkg.name ? (
                    <Check className="h-3.5 w-3.5 text-emerald-400" />
                  ) : (
                    <Terminal className="h-3.5 w-3.5" />
                  )}
                  {copiedId === pkg.name ? "Copied Command" : `lpp add ${pkg.name}`}
                </button>
              </div>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
