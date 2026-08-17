import { useState, useEffect, useMemo } from "react";
import { motion, AnimatePresence } from "framer-motion";
import {
  X,
  Search,
  Package,
  Key,
  Terminal,
  ExternalLink,
  Check,
  Copy,
  ShieldCheck,
  Sparkles,
  RefreshCw,
  SlidersHorizontal,
  Layers,
  ArrowUpDown,
  BookOpen,
  GitBranch,
  Calendar,
  User,
} from "lucide-react";
import { rankPackages, PackageItem, SortMode } from "../lib/searchAlgorithm";

interface RegistryModalProps {
  isOpen: boolean;
  onClose: () => void;
  onOpenAuth?: () => void;
}

const CATEGORIES = [
  { id: "all", label: "All Packages" },
  { id: "data-structures", label: "Data Structures" },
  { id: "algorithms", label: "Algorithms" },
  { id: "network", label: "Networking" },
  { id: "graphics", label: "Graphics & UI" },
  { id: "web", label: "Lreact & Web" },
  { id: "tools", label: "Dev Tools" },
];

export default function RegistryModal({ isOpen, onClose, onOpenAuth }: RegistryModalProps) {
  const [tab, setTab] = useState<"explore" | "publish">("explore");
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState("all");
  const [sortMode, setSortMode] = useState<SortMode>("relevance");
  const [rawPackages, setRawPackages] = useState<PackageItem[]>([]);
  const [selectedPackage, setSelectedPackage] = useState<PackageItem | null>(null);
  const [loading, setLoading] = useState(false);
  const [copiedText, setCopiedText] = useState<string | null>(null);

  // Seed default official packages for instant browsing + fetch live registry
  useEffect(() => {
    if (isOpen) {
      fetchRegistry();
    }
  }, [isOpen]);

  const fetchRegistry = async () => {
    setLoading(true);
    try {
      const res = await fetch("https://registry.lplusplus.bond/index.json");
      if (res.ok) {
        const data = await res.json();
        let list: PackageItem[] = [];
        if (data.packages) {
          list = Object.values(data.packages);
        } else if (data.results) {
          list = data.results;
        }

        // Add standard core libraries if registry is fresh
        if (list.length === 0) {
          list = [
            {
              name: "lpp-graph",
              version: "1.0.0",
              description: "Weighted directed graph with Dijkstra shortest-path and Kahn topological sort. Pure L++.",
              authors: ["samarnever-droid"],
              keywords: ["algorithms", "graph", "dijkstra", "data-structures"],
              downloads: 0,
              updated_at: "2026-08-17T10:00:00Z",
              owner: "samarnever-droid@lplusplus.bond",
              license: "MIT",
              dependencies: [],
            },
            {
              name: "lreact",
              version: "1.2.0",
              description: "Declarative reactive UI framework and desktop web application runtime for L++.",
              authors: ["L++ Core Team"],
              keywords: ["web", "ui", "react", "desktop", "graphics"],
              downloads: 0,
              updated_at: "2026-08-16T14:30:00Z",
              owner: "core@lplusplus.bond",
              license: "Apache-2.0",
              dependencies: ["lpp-json", "lpp-http"],
            },
            {
              name: "lpp-json",
              version: "0.9.4",
              description: "Zero-allocation SIMD-accelerated JSON parser and serializer with native L++ structs.",
              authors: ["L++ Performance Working Group"],
              keywords: ["data-structures", "tools", "serialization"],
              downloads: 0,
              updated_at: "2026-08-15T09:12:00Z",
              owner: "wg-perf@lplusplus.bond",
              license: "MIT",
              dependencies: [],
            },
            {
              name: "lpp-http",
              version: "1.0.2",
              description: "High-concurrency async HTTP/1.1 & HTTP/2 client and server engine built on epoll/IOCP.",
              authors: ["L++ Network Group"],
              keywords: ["network", "web", "async", "server"],
              downloads: 0,
              updated_at: "2026-08-14T18:40:00Z",
              owner: "net@lplusplus.bond",
              license: "MIT",
              dependencies: [],
            },
            {
              name: "@samarnever-droid/lpp-zip",
              version: "0.3.1",
              description: "DEFLATE compression, streaming zip extraction and tarball packager for L++ tooling.",
              authors: ["samarnever-droid"],
              keywords: ["tools", "compression", "data-structures"],
              downloads: 0,
              updated_at: "2026-08-13T11:20:00Z",
              owner: "samarnever-droid@lplusplus.bond",
              license: "MIT",
              dependencies: [],
            },
          ];
        }
        setRawPackages(list);
      }
    } catch {
      // Fallback
    } finally {
      setLoading(false);
    }
  };

  const filteredPackages = useMemo(() => {
    return rankPackages(rawPackages, query, category, sortMode);
  }, [rawPackages, query, category, sortMode]);

  const copyToClipboard = (text: string, id: string) => {
    navigator.clipboard.writeText(text);
    setCopiedText(id);
    setTimeout(() => setCopiedText(null), 2000);
  };

  if (!isOpen) return null;

  return (
    <AnimatePresence>
      <div className="fixed inset-0 z-50 flex items-center justify-center p-3 sm:p-6 md:p-10">
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          onClick={onClose}
          className="fixed inset-0 bg-black/85 backdrop-blur-md"
        />

        <motion.div
          initial={{ scale: 0.95, opacity: 0, y: 20 }}
          animate={{ scale: 1, opacity: 1, y: 0 }}
          exit={{ scale: 0.95, opacity: 0, y: 20 }}
          className="relative flex flex-col w-full max-w-5xl h-[88vh] overflow-hidden rounded-2xl border border-white/15 bg-[#090c10] shadow-2xl"
        >
          {/* Top Bar */}
          <div className="flex items-center justify-between border-b border-white/10 px-6 py-4">
            <div className="flex items-center gap-3">
              <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-acid/10 border border-acid/30 text-acid">
                <Package className="h-5 w-5" />
              </div>
              <div>
                <h2 className="text-lg font-bold text-white flex items-center gap-2">
                  L++ Package Registry
                  <span className="rounded-full bg-emerald-500/20 px-2 py-0.5 font-mono text-[10px] text-emerald-400 border border-emerald-500/30">
                    registry.lplusplus.bond
                  </span>
                </h2>
                <p className="text-xs text-white/50 font-mono">
                  Smart fuzzy search &bull; Ownership locking &bull; Zero-leakage storage
                </p>
              </div>
            </div>

            <div className="flex items-center gap-3">
              <button
                onClick={() => {
                  onClose();
                  onOpenAuth?.();
                }}
                className="hidden sm:flex items-center gap-1.5 rounded-lg border border-acid/40 bg-acid/10 px-3 py-1.5 font-mono text-xs font-bold text-acid hover:bg-acid/20"
              >
                <Key className="h-3.5 w-3.5" />
                Publisher Dashboard & Auth
              </button>

              <button
                onClick={onClose}
                className="grid h-8 w-8 place-items-center rounded-lg border border-white/10 text-white/60 hover:text-white hover:border-white/30"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
          </div>

          {/* Search Controls */}
          <div className="border-b border-white/10 bg-white/[0.02] p-4 space-y-3">
            <div className="flex flex-col sm:flex-row gap-3 items-center">
              <div className="relative flex-1 w-full">
                <Search className="absolute left-3.5 top-1/2 -translate-y-1/2 h-4 w-4 text-white/40" />
                <input
                  type="text"
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  placeholder="Search packages by name, description, tags (e.g. lreact, graph, dijkstra, json)..."
                  className="w-full rounded-xl border border-white/15 bg-white/[0.04] pl-10 pr-10 py-2.5 font-mono text-sm text-white placeholder:text-white/30 focus:border-acid/60 focus:outline-none"
                />
                {loading && (
                  <RefreshCw className="absolute right-3.5 top-1/2 -translate-y-1/2 h-4 w-4 animate-spin text-acid" />
                )}
              </div>

              <div className="flex items-center gap-2 self-end sm:self-center">
                <div className="flex items-center gap-1.5 rounded-xl border border-white/10 bg-white/[0.03] px-3 py-2 text-xs font-mono text-white/70">
                  <ArrowUpDown className="h-3.5 w-3.5 text-white/40" />
                  <select
                    value={sortMode}
                    onChange={(e) => setSortMode(e.target.value as SortMode)}
                    className="bg-transparent text-white focus:outline-none cursor-pointer"
                  >
                    <option value="relevance" className="bg-[#090c10]">Best Relevance</option>
                    <option value="downloads" className="bg-[#090c10]">Most Downloads</option>
                    <option value="recent" className="bg-[#090c10]">Recently Updated</option>
                    <option value="name" className="bg-[#090c10]">Alphabetical A-Z</option>
                  </select>
                </div>
              </div>
            </div>

            {/* Category Pills */}
            <div className="flex items-center gap-1.5 overflow-x-auto pb-1 text-xs font-mono">
              {CATEGORIES.map((c) => (
                <button
                  key={c.id}
                  onClick={() => setCategory(c.id)}
                  className={`px-3 py-1 rounded-lg transition-colors whitespace-nowrap ${
                    category === c.id
                      ? "bg-acid text-ink font-semibold"
                      : "bg-white/5 text-white/60 hover:text-white hover:bg-white/10"
                  }`}
                >
                  {c.label}
                </button>
              ))}
            </div>
          </div>

          {/* Main Content Split: Package List & Package Detail */}
          <div className="flex-1 overflow-hidden grid grid-cols-1 md:grid-cols-12">
            {/* Left: Package List */}
            <div className={`p-4 overflow-y-auto space-y-2.5 ${selectedPackage ? "hidden md:block md:col-span-6 border-r border-white/10" : "col-span-12"}`}>
              <div className="flex items-center justify-between text-xs font-mono text-white/40 px-1 mb-1">
                <span>FOUND {filteredPackages.length} PACKAGES</span>
                <span>SORT: {sortMode.toUpperCase()}</span>
              </div>

              {filteredPackages.length === 0 ? (
                <div className="rounded-xl border border-dashed border-white/10 p-10 text-center space-y-3">
                  <Package className="mx-auto h-8 w-8 text-white/20" />
                  <p className="text-sm text-white/70">No packages match your search query.</p>
                  <button
                    onClick={() => {
                      setQuery("");
                      setCategory("all");
                    }}
                    className="text-xs font-mono text-acid hover:underline"
                  >
                    Clear search filters
                  </button>
                </div>
              ) : (
                filteredPackages.map((pkg) => (
                  <div
                    key={`${pkg.name}-${pkg.version}`}
                    onClick={() => setSelectedPackage(pkg)}
                    className={`cursor-pointer rounded-xl border p-4 transition-all ${
                      selectedPackage?.name === pkg.name
                        ? "border-acid bg-acid/[0.04]"
                        : "border-white/10 bg-white/[0.02] hover:border-white/25 hover:bg-white/[0.04]"
                    }`}
                  >
                    <div className="flex items-start justify-between gap-2">
                      <div>
                        <div className="flex items-center gap-2">
                          <h3 className="font-mono text-base font-bold text-white group-hover:text-acid">
                            {pkg.name}
                          </h3>
                          <span className="rounded bg-acid/10 px-2 py-0.5 font-mono text-xs text-acid border border-acid/20">
                            v{pkg.version}
                          </span>
                        </div>
                        <p className="text-xs text-white/60 mt-1 line-clamp-2">
                          {pkg.description || "High-performance L++ library"}
                        </p>
                      </div>

                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          copyToClipboard(`lpp add ${pkg.name}`, `copy-${pkg.name}`);
                        }}
                        className="flex items-center gap-1 rounded-lg border border-white/10 bg-white/5 px-2.5 py-1 text-[11px] font-mono text-white/70 hover:border-acid/40 hover:text-acid shrink-0"
                      >
                        {copiedText === `copy-${pkg.name}` ? (
                          <Check className="h-3 w-3 text-emerald-400" />
                        ) : (
                          <Copy className="h-3 w-3" />
                        )}
                        {copiedText === `copy-${pkg.name}` ? "Added" : "lpp add"}
                      </button>
                    </div>

                    <div className="mt-3 flex flex-wrap items-center gap-3 text-[11px] font-mono text-white/40">
                      {pkg.owner && <span>by {pkg.owner.split("@")[0]}</span>}
                      {pkg.downloads && <span>&bull; {pkg.downloads.toLocaleString()} installs</span>}
                      {pkg.license && <span>&bull; {pkg.license}</span>}
                    </div>
                  </div>
                ))
              )}
            </div>

            {/* Right: Package Details Inspector */}
            {selectedPackage && (
              <div className="col-span-12 md:col-span-6 p-6 overflow-y-auto space-y-6 bg-white/[0.01]">
                <div className="flex items-start justify-between">
                  <div>
                    <h2 className="text-xl font-bold font-mono text-white flex items-center gap-2">
                      {selectedPackage.name}
                      <span className="rounded-md bg-acid/10 px-2 py-0.5 text-xs text-acid border border-acid/30">
                        v{selectedPackage.version}
                      </span>
                    </h2>
                    <p className="text-xs text-white/60 mt-1">
                      {selectedPackage.description}
                    </p>
                  </div>
                  <button
                    onClick={() => setSelectedPackage(null)}
                    className="md:hidden p-1 text-white/50 hover:text-white"
                  >
                    <X className="h-5 w-5" />
                  </button>
                </div>

                {/* Installation Commands */}
                <div className="space-y-2">
                  <span className="text-xs font-mono uppercase tracking-widest text-white/50 block">
                    Install Package
                  </span>
                  <div className="flex items-center justify-between gap-2 rounded-xl border border-white/10 bg-black/60 px-3.5 py-2.5 font-mono text-xs">
                    <span className="text-acid">lpp add {selectedPackage.name}</span>
                    <button
                      onClick={() => copyToClipboard(`lpp add ${selectedPackage.name}`, "detail-install")}
                      className="flex items-center gap-1 text-white/60 hover:text-white"
                    >
                      {copiedText === "detail-install" ? <Check className="h-3.5 w-3.5 text-emerald-400" /> : <Copy className="h-3.5 w-3.5" />}
                      {copiedText === "detail-install" ? "Copied" : "Copy"}
                    </button>
                  </div>
                </div>

                {/* Metadata Grid */}
                <div className="grid grid-cols-2 gap-3 text-xs font-mono">
                  <div className="rounded-xl border border-white/10 bg-white/[0.02] p-3">
                    <span className="text-white/40 block mb-0.5">License</span>
                    <span className="text-white font-bold">{selectedPackage.license || "MIT"}</span>
                  </div>
                  <div className="rounded-xl border border-white/10 bg-white/[0.02] p-3">
                    <span className="text-white/40 block mb-0.5">Publisher</span>
                    <span className="text-white font-bold truncate block">{selectedPackage.owner || "L++ Community"}</span>
                  </div>
                </div>

                {/* Keywords */}
                {selectedPackage.keywords && selectedPackage.keywords.length > 0 && (
                  <div>
                    <span className="text-xs font-mono uppercase tracking-widest text-white/50 block mb-2">
                      Tags & Keywords
                    </span>
                    <div className="flex flex-wrap gap-1.5">
                      {selectedPackage.keywords.map((k) => (
                        <span
                          key={k}
                          className="text-xs font-mono text-acid bg-acid/10 border border-acid/20 px-2 py-0.5 rounded"
                        >
                          #{k}
                        </span>
                      ))}
                    </div>
                  </div>
                )}

                {/* Dependencies */}
                {selectedPackage.dependencies && selectedPackage.dependencies.length > 0 && (
                  <div>
                    <span className="text-xs font-mono uppercase tracking-widest text-white/50 block mb-2">
                      Dependencies ({selectedPackage.dependencies.length})
                    </span>
                    <div className="space-y-1">
                      {selectedPackage.dependencies.map((dep) => (
                        <div
                          key={dep}
                          className="flex items-center gap-2 rounded-lg border border-white/10 bg-white/[0.02] px-3 py-1.5 font-mono text-xs text-white/80"
                        >
                          <GitBranch className="h-3 w-3 text-acid" />
                          {dep}
                        </div>
                      ))}
                    </div>
                  </div>
                )}

                {/* Integrity & Immutability */}
                <div className="rounded-xl border border-white/10 bg-white/[0.02] p-4 text-xs font-mono space-y-1 text-white/60">
                  <div className="flex items-center gap-2 text-white">
                    <ShieldCheck className="h-4 w-4 text-emerald-400" />
                    <span>Verified Registry Integrity</span>
                  </div>
                  <p className="text-white/40 text-[11px]">
                    Downloads streamed securely with SHA-256 validation via registry.lplusplus.bond
                  </p>
                </div>
              </div>
            )}
          </div>
        </motion.div>
      </div>
    </AnimatePresence>
  );
}
