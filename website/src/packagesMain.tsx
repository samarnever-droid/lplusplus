import React, { useState, useEffect, useMemo } from "react";
import ReactDOM from "react-dom/client";
import {
  ClerkProvider,
  SignedIn,
  SignedOut,
  SignInButton,
  UserButton,
} from "@clerk/clerk-react";
import {
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
  ArrowUpDown,
  BookOpen,
  GitBranch,
  Calendar,
  User,
  ArrowLeft,
  Filter,
  X,
  Layers,
  Code,
} from "lucide-react";
import { rankPackages, PackageItem, SortMode } from "./lib/searchAlgorithm";
import { CLERK_PUBLISHABLE_KEY, clerkAppearance } from "./lib/clerk";
import Footer from "./components/Footer";
import "./index.css";

const CATEGORIES = [
  { id: "all", label: "All Packages" },
  { id: "algorithms", label: "Algorithms" },
  { id: "web", label: "Web & Lreact" },
  { id: "database", label: "Databases" },
  { id: "tools", label: "Dev Tools" },
  { id: "compression", label: "Compression" },
  { id: "crypto", label: "Crypto & Security" },
  { id: "data-structures", label: "Data Structures" },
];

export function PackagesApp() {
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState("all");
  const [sortMode, setSortMode] = useState<SortMode>("relevance");
  const [packages, setPackages] = useState<PackageItem[]>([]);
  const [selectedPackage, setSelectedPackage] = useState<PackageItem | null>(null);
  const [loading, setLoading] = useState(true);
  const [stats, setStats] = useState({
    packages_count: 17,
    downloads_count: 0,
    versions_count: 17,
    publishers_count: 1,
  });

  useEffect(() => {
    fetchPackages();
    fetchStats();
  }, []);

  const fetchStats = async () => {
    try {
      const res = await fetch("https://registry.lplusplus.bond/stats");
      if (res.ok) {
        const data = await res.json();
        if (data && typeof data.packages_count === "number") {
          setStats({
            packages_count: data.packages_count,
            downloads_count: data.downloads_count || 0,
            versions_count: data.versions_count || data.packages_count,
            publishers_count: data.publishers_count || 1,
          });
        }
      }
    } catch {
      // ignore
    }
  };

  const fetchPackages = async () => {
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
        setPackages(list);
      }
    } catch {
      // ignore
    } finally {
      setLoading(false);
    }
  };

  const filtered = useMemo(() => {
    return rankPackages(packages, query, category, sortMode);
  }, [packages, query, category, sortMode]);

  const copy = (text: string, id: string) => {
    navigator.clipboard.writeText(text);
    setCopiedText(id);
    setTimeout(() => setCopiedText(null), 2000);
  };

  return (
    <div className="min-h-screen bg-[#07090d] text-white flex flex-col font-sans antialiased">
      {/* Top Header */}
      <header className="sticky top-0 z-40 border-b border-white/10 bg-[#07090d]/90 backdrop-blur-xl">
        <div className="mx-auto flex h-16 max-w-7xl items-center justify-between px-5 md:px-8">
          <div className="flex items-center gap-4">
            <a href="/" className="flex items-center gap-2.5 text-white/80 hover:text-white">
              <ArrowLeft className="h-4 w-4" />
              <span className="font-mono text-xs">Back to Main</span>
            </a>
            <div className="h-4 w-[1px] bg-white/20" />
            <div className="flex items-center gap-2">
              <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-acid/10 text-acid font-bold border border-acid/20">
                <Package className="h-4 w-4" />
              </div>
              <span className="font-mono text-sm font-bold tracking-wider">L++ REGISTRY</span>
            </div>
          </div>

          <div className="flex items-center gap-3">
            <SignedIn>
              <a
                href="/account.html"
                className="flex items-center gap-1.5 rounded-lg border border-acid/40 bg-acid/10 px-3.5 py-1.5 font-mono text-xs font-bold text-acid hover:bg-acid/20"
              >
                <Key className="h-3.5 w-3.5" />
                Developer Account
              </a>
              <UserButton
                afterSignOutUrl="/packages.html"
                appearance={{
                  elements: {
                    avatarBox: "h-8 w-8 rounded-lg border border-acid/40",
                  },
                }}
              />
            </SignedIn>
            <SignedOut>
              <a
                href="/account.html"
                className="flex items-center gap-1.5 rounded-lg border border-white/15 bg-white/5 px-3.5 py-1.5 font-mono text-xs text-white hover:border-acid/40 hover:text-acid"
              >
                <Key className="h-3.5 w-3.5 text-acid" />
                Sign In / Token
              </a>
            </SignedOut>
          </div>
        </div>
      </header>

      {/* Hero Search Section */}
      <section className="border-b border-white/10 bg-gradient-to-b from-white/[0.03] to-transparent py-14 px-5">
        <div className="mx-auto max-w-4xl text-center space-y-4">
          <div className="flex flex-wrap items-center justify-center gap-2.5">
            <span className="rounded-full border border-acid/30 bg-acid/10 px-3.5 py-1 font-mono text-xs text-acid inline-flex items-center gap-1.5 shadow-[0_0_15px_rgba(200,241,75,0.2)]">
              <span className="h-2 w-2 rounded-full bg-acid animate-pulse" />
              Live Hub &bull; {stats.packages_count} Verified Packages
            </span>
            <span className="rounded-full border border-emerald-400/30 bg-emerald-400/10 px-3.5 py-1 font-mono text-xs text-emerald-400 inline-flex items-center gap-1.5">
              <ShieldCheck className="h-3.5 w-3.5" />
              {stats.downloads_count > 0 ? `${stats.downloads_count.toLocaleString()} Real Downloads` : "Real-Time Registry"}
            </span>
            <span className="rounded-full border border-white/20 bg-white/5 px-3.5 py-1 font-mono text-xs text-white/70 inline-flex items-center gap-1.5">
              <Key className="h-3 w-3 text-acid" />
              {stats.publishers_count} Official Publisher{stats.publishers_count > 1 ? "s" : ""}
            </span>
          </div>
          <h1 className="text-3xl sm:text-5xl font-black font-mono tracking-tight text-white leading-tight">
            Explore & Publish <span className="text-acid">L++ Packages</span>
          </h1>
          <p className="text-sm sm:text-base text-white/60 font-mono max-w-2xl mx-auto">
            High-performance libraries streamed securely through <code className="text-white">registry.lplusplus.bond</code> with SemVer immutability and SHA-256 verification.
          </p>

          <div className="pt-4 max-w-2xl mx-auto">
            <div className="relative">
              <Search className="absolute left-4 top-1/2 -translate-y-1/2 h-5 w-5 text-white/40" />
              <input
                type="text"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search packages (e.g. lreact, sqlite, graph, json, zip)..."
                className="w-full rounded-2xl border border-white/20 bg-white/[0.05] pl-12 pr-12 py-3.5 font-mono text-sm sm:text-base text-white placeholder:text-white/30 focus:border-acid/60 focus:outline-none shadow-2xl backdrop-blur-md"
              />
              {loading && (
                <RefreshCw className="absolute right-4 top-1/2 -translate-y-1/2 h-5 w-5 animate-spin text-acid" />
              )}
            </div>
          </div>
        </div>
      </section>

      {/* Main Catalog View */}
      <main className="mx-auto max-w-7xl w-full flex-1 px-5 md:px-8 py-10">
        <div className="flex flex-col md:flex-row gap-8">
          {/* Left Sidebar: Categories & Filters */}
          <aside className="w-full md:w-64 shrink-0 space-y-6">
            <div>
              <h3 className="text-xs font-mono uppercase tracking-widest text-white/50 mb-3 flex items-center gap-2">
                <Filter className="h-3.5 w-3.5" />
                Categories
              </h3>
              <div className="space-y-1 font-mono text-xs">
                {CATEGORIES.map((c) => (
                  <button
                    key={c.id}
                    onClick={() => setCategory(c.id)}
                    className={`flex w-full items-center justify-between rounded-xl px-3.5 py-2 transition-all ${
                      category === c.id
                        ? "bg-acid text-ink font-bold shadow-[0_0_15px_rgba(200,241,75,0.3)]"
                        : "text-white/70 hover:bg-white/5 hover:text-white"
                    }`}
                  >
                    <span>{c.label}</span>
                  </button>
                ))}
              </div>
            </div>

            <div>
              <h3 className="text-xs font-mono uppercase tracking-widest text-white/50 mb-3 flex items-center gap-2">
                <ArrowUpDown className="h-3.5 w-3.5" />
                Sort By
              </h3>
              <select
                value={sortMode}
                onChange={(e) => setSortMode(e.target.value as SortMode)}
                className="w-full rounded-xl border border-white/15 bg-white/[0.03] px-3.5 py-2.5 font-mono text-xs text-white focus:outline-none"
              >
                <option value="relevance" className="bg-[#07090d]">Best Relevance</option>
                <option value="downloads" className="bg-[#07090d]">Most Downloads</option>
                <option value="recent" className="bg-[#07090d]">Recently Published</option>
                <option value="name" className="bg-[#07090d]">Alphabetical (A-Z)</option>
              </select>
            </div>

            <div className="rounded-2xl border border-white/10 bg-white/[0.02] p-4 text-xs font-mono space-y-2.5 text-white/60">
              <span className="font-bold text-white block flex items-center gap-1.5">
                <Terminal className="h-3.5 w-3.5 text-acid" />
                CLI Publishing
              </span>
              <p className="text-[11px] text-white/40">
                Log in and publish packages natively with the L++ CLI:
              </p>
              <pre className="rounded-lg bg-black/60 p-2.5 text-[10px] text-acid overflow-x-auto border border-white/10">
                lpp login lpp_pub_...<br />lpp publish
              </pre>
            </div>
          </aside>

          {/* Right: Package List */}
          <div className="flex-1 space-y-4">
            <div className="flex items-center justify-between text-xs font-mono text-white/40 border-b border-white/10 pb-3">
              <span>SHOWING {filtered.length} PACKAGES</span>
              <span>HOSTED AT REGISTRY.LPLUSPLUS.BOND</span>
            </div>

            {filtered.length === 0 ? (
              <div className="rounded-2xl border border-dashed border-white/10 p-16 text-center space-y-3">
                <Package className="mx-auto h-10 w-10 text-white/20" />
                <p className="text-base text-white/70">No packages found matching your query.</p>
                <button
                  onClick={() => {
                    setQuery("");
                    setCategory("all");
                  }}
                  className="font-mono text-xs text-acid hover:underline"
                >
                  Reset all filters
                </button>
              </div>
            ) : (
              <div className="grid grid-cols-1 gap-4">
                {filtered.map((pkg) => (
                  <div
                    key={`${pkg.name}-${pkg.version}`}
                    className="flex flex-col sm:flex-row items-start sm:items-center justify-between gap-4 rounded-2xl border border-white/10 bg-white/[0.02] p-6 transition-all duration-300 hover:border-acid/40 hover:bg-white/[0.04] group cursor-pointer"
                    onClick={() => setSelectedPackage(pkg)}
                  >
                    <div className="space-y-2 flex-1">
                      <div className="flex flex-wrap items-center gap-3">
                        <span className="font-mono text-lg font-bold text-white group-hover:text-acid transition-colors">
                          {pkg.name}
                        </span>
                        <span className="rounded-md bg-acid/10 px-2.5 py-0.5 font-mono text-xs font-semibold text-acid border border-acid/20">
                          v{pkg.version}
                        </span>
                        {pkg.license && (
                          <span className="font-mono text-xs text-white/40 border border-white/10 px-2 py-0.5 rounded">
                            {pkg.license}
                          </span>
                        )}
                        <span className="font-mono text-[11px] text-emerald-400 flex items-center gap-1 bg-emerald-500/10 px-2 py-0.5 rounded border border-emerald-500/20">
                          <ShieldCheck className="h-3 w-3" />
                          Verified
                        </span>
                      </div>

                      <p className="text-xs sm:text-sm text-white/65 max-w-2xl leading-relaxed">
                        {pkg.description || "High-performance native L++ library."}
                      </p>

                      <div className="flex flex-wrap items-center gap-4 text-xs font-mono text-white/40 pt-1">
                        {pkg.owner && <span>by {pkg.owner.split("@")[0]}</span>}
                        {pkg.downloads && <span>&bull; {pkg.downloads.toLocaleString()} downloads</span>}
                        {pkg.keywords && pkg.keywords.length > 0 && (
                          <div className="flex flex-wrap gap-1.5">
                            {pkg.keywords.map((k) => (
                              <span key={k} className="text-white/35 group-hover:text-white/60">
                                #{k}
                              </span>
                            ))}
                          </div>
                        )}
                      </div>
                    </div>

                    <div
                      className="flex items-center gap-2 self-end sm:self-center shrink-0"
                      onClick={(e) => e.stopPropagation()}
                    >
                      <button
                        onClick={() => copy(`lpp add ${pkg.name}`, `add-${pkg.name}`)}
                        className="flex items-center gap-1.5 rounded-xl border border-white/15 bg-white/5 px-4 py-2 font-mono text-xs font-bold text-white hover:border-acid/50 hover:text-acid transition-all"
                      >
                        {copiedText === `add-${pkg.name}` ? (
                          <Check className="h-3.5 w-3.5 text-emerald-400" />
                        ) : (
                          <Terminal className="h-3.5 w-3.5" />
                        )}
                        {copiedText === `add-${pkg.name}` ? "Copied" : `lpp add ${pkg.name}`}
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      </main>

      {/* Package Detail Inspector Modal */}
      {selectedPackage && (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/80 backdrop-blur-md">
          <div className="w-full max-w-2xl rounded-2xl border border-white/15 bg-[#0b0e14] p-6 space-y-6 shadow-2xl max-h-[90vh] overflow-y-auto">
            <div className="flex items-start justify-between border-b border-white/10 pb-4">
              <div className="space-y-1">
                <div className="flex items-center gap-3">
                  <h2 className="font-mono text-2xl font-bold text-white">{selectedPackage.name}</h2>
                  <span className="rounded-md bg-acid/10 px-2.5 py-0.5 font-mono text-xs font-semibold text-acid border border-acid/30">
                    v{selectedPackage.version}
                  </span>
                </div>
                <p className="text-xs font-mono text-white/50">
                  Published by {selectedPackage.owner || "L++ Core Team"} &bull; {selectedPackage.license || "MIT"} License
                </p>
              </div>
              <button
                onClick={() => setSelectedPackage(null)}
                className="rounded-lg p-1.5 text-white/50 hover:bg-white/10 hover:text-white"
              >
                <X className="h-5 w-5" />
              </button>
            </div>

            <div className="space-y-4">
              <div>
                <span className="text-xs font-mono uppercase tracking-wider text-white/40 block mb-1">
                  Installation
                </span>
                <div className="flex items-center justify-between gap-3 rounded-xl border border-white/15 bg-black p-3 font-mono text-xs">
                  <code className="text-acid">lpp add {selectedPackage.name}</code>
                  <button
                    onClick={() => copy(`lpp add ${selectedPackage.name}`, "modal-copy")}
                    className="flex items-center gap-1 text-white/60 hover:text-white"
                  >
                    {copiedText === "modal-copy" ? <Check className="h-3.5 w-3.5 text-emerald-400" /> : <Copy className="h-3.5 w-3.5" />}
                    {copiedText === "modal-copy" ? "Copied" : "Copy"}
                  </button>
                </div>
              </div>

              <div>
                <span className="text-xs font-mono uppercase tracking-wider text-white/40 block mb-1">
                  Description
                </span>
                <p className="text-sm text-white/70 leading-relaxed">
                  {selectedPackage.description || "High-performance native library for L++."}
                </p>
              </div>

              {selectedPackage.keywords && selectedPackage.keywords.length > 0 && (
                <div>
                  <span className="text-xs font-mono uppercase tracking-wider text-white/40 block mb-2">
                    Keywords & Tags
                  </span>
                  <div className="flex flex-wrap gap-1.5">
                    {selectedPackage.keywords.map((k) => (
                      <span key={k} className="rounded-lg bg-white/5 px-2.5 py-1 font-mono text-xs text-white/70 border border-white/10">
                        #{k}
                      </span>
                    ))}
                  </div>
                </div>
              )}

              <div className="pt-2 border-t border-white/10 flex items-center justify-between font-mono text-xs text-white/40">
                <span>Downloads: {selectedPackage.downloads && selectedPackage.downloads > 0 ? selectedPackage.downloads.toLocaleString() : "0"}</span>
                <span>Registry: registry.lplusplus.bond</span>
              </div>
            </div>
          </div>
        </div>
      )}

      <Footer />
    </div>
  );
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ClerkProvider
      publishableKey={CLERK_PUBLISHABLE_KEY}
      appearance={clerkAppearance}
    >
      <PackagesApp />
    </ClerkProvider>
  </React.StrictMode>
);
