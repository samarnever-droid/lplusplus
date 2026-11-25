import { useEffect, useMemo, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  ArrowRight,
  BookOpen,
  Code2,
  Package,
  Search,
  Shield,
  Sparkles,
  Terminal,
  X,
} from "lucide-react";

interface PaletteItem {
  id: string;
  title: string;
  subtitle: string;
  category: "Pages" | "Keel CLI" | "Language v0.3.2" | "Stdlib";
  href?: string;
  copy?: string;
}

const ITEMS: PaletteItem[] = [
  { id: "home", title: "Home", subtitle: "L++ overview & benchmarks", category: "Pages", href: "/" },
  { id: "playground", title: "Interactive L++ Playground", subtitle: "Write, share (#play=...), and save snippets to your account", category: "Pages", href: "/#playground" },
  { id: "packages", title: "Packages Registry", subtitle: "Browse live packages & manage bookmarks", category: "Pages", href: "/packages.html" },
  { id: "academy", title: "L++ Academy", subtitle: "Interactive systems programming lessons", category: "Pages", href: "/academy.html" },
  { id: "studio", title: "Publisher Studio", subtitle: "Manifest builder, Teams & Orgs, Passkeys & MFA", category: "Pages", href: "/account.html" },
  { id: "install", title: "Install L++ & Keel", subtitle: "curl -fsSL https://lplusplus.bond/install.sh | sh", category: "Pages", href: "/#install" },

  { id: "k-new", title: "keel new <name>", subtitle: "Create a new L++ project with Keel.toml & src/main.lpp", category: "Keel CLI", copy: "keel new my-app" },
  { id: "k-add", title: "keel add <pkg>", subtitle: "Add a registry dependency (or --path ../lib)", category: "Keel CLI", copy: "keel add lpp-semver" },
  { id: "k-install", title: "keel install", subtitle: "Resolve and lock all dependencies into Keel.lock", category: "Keel CLI", copy: "keel install" },
  { id: "k-build", title: "keel build && keel run", subtitle: "Incremental native build & execution", category: "Keel CLI", copy: "keel build && keel run" },
  { id: "k-search", title: "keel search <query>", subtitle: "Search packages in the canonical git registry", category: "Keel CLI", copy: "keel search lpp" },
  { id: "k-publish", title: "keel publish", subtitle: "Pack SHA-256 tarball, commit, and push release", category: "Keel CLI", copy: "keel publish" },
  { id: "k-verify", title: "keel verify", subtitle: "Audit SHA-256 checksums of all locked artifacts", category: "Keel CLI", copy: "keel verify" },

  { id: "l-pure", title: "@pure & @deny(fs, net, exec, ffi, unsafe)", subtitle: "Compile-time transitive capability sandboxing (zero runtime cost)", category: "Language v0.3.2", href: "/#playground" },
  { id: "l-slice", title: "String Slicing: s[a..b], s[a..=b], s[..b], s[a..]", subtitle: "Desugared to str_substr across MIR, Cranelift, LLVM, WASM", category: "Language v0.3.2", href: "/#playground" },
  { id: "l-fn", title: "Function Types: fn(Int, Int) -> Int", subtitle: "First-class function & closure type annotations", category: "Language v0.3.2", href: "/#syntax" },
  { id: "l-ternary", title: "Ternary: a if cond else b", subtitle: "Short-circuiting conditional expressions", category: "Language v0.3.2", href: "/#playground" },
  { id: "l-power", title: "Power & Bitwise: **, **=, ~, &=, |=, ^=, <<=, >>=", subtitle: "Integer & float exponentiation plus MIR algebraic simplification", category: "Language v0.3.2", href: "/#playground" },
  { id: "l-kernel", title: "Bare-Metal Kernels (--target x86_64-unknown-none)", subtitle: "Ptr[T], volatile MMIO, port I/O, @export_name, --multiboot2", category: "Language v0.3.2", href: "/#playground" },

  { id: "s-webview", title: "webview_window_create(title, w, h, flags)", subtitle: "Native OS WebView2 / WKWebView / WebKitGTK desktop GUI", category: "Stdlib", href: "/#stdlib" },
  { id: "s-file", title: "file_open(path, flags)", subtitle: "Bitflags: 1=O_SYNC, 2=O_EXCL, 4=truncate, 8=read-only", category: "Stdlib", href: "/#stdlib" },
  { id: "s-substr", title: "str_substr(s, start, len)", subtitle: "Bounds-clamped UTF-8 substring extraction", category: "Stdlib", href: "/#stdlib" },
];

export default function CommandPalette({
  isOpen,
  onClose,
}: {
  isOpen: boolean;
  onClose: () => void;
}) {
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const [copiedId, setCopiedId] = useState<string | null>(null);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return ITEMS;
    return ITEMS.filter(
      (i) =>
        i.title.toLowerCase().includes(q) ||
        i.subtitle.toLowerCase().includes(q) ||
        i.category.toLowerCase().includes(q)
    );
  }, [query]);

  useEffect(() => {
    setSelected(0);
  }, [query, isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        onClose();
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        setSelected((s) => (filtered.length ? (s + 1) % filtered.length : 0));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setSelected((s) => (filtered.length ? (s - 1 + filtered.length) % filtered.length : 0));
      } else if (e.key === "Enter" && filtered[selected]) {
        e.preventDefault();
        activate(filtered[selected]);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [isOpen, filtered, selected, onClose]);

  const activate = (item: PaletteItem) => {
    if (item.copy) {
      navigator.clipboard.writeText(item.copy);
      setCopiedId(item.id);
      setTimeout(() => {
        setCopiedId(null);
        onClose();
      }, 650);
    } else if (item.href) {
      onClose();
      window.location.href = item.href;
    }
  };

  const iconFor = (cat: PaletteItem["category"]) => {
    switch (cat) {
      case "Pages":
        return <Sparkles className="h-4 w-4 text-[#a78bfa]" />;
      case "Keel CLI":
        return <Terminal className="h-4 w-4 text-emerald-400" />;
      case "Language v0.3.2":
        return <Shield className="h-4 w-4 text-sky-400" />;
      case "Stdlib":
        return <BookOpen className="h-4 w-4 text-amber-400" />;
    }
  };

  return (
    <AnimatePresence>
      {isOpen && (
        <div className="fixed inset-0 z-[120] flex items-start justify-center pt-[12vh] px-4">
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            onClick={onClose}
            className="absolute inset-0 bg-black/75 backdrop-blur-sm"
          />
          <motion.div
            initial={{ opacity: 0, scale: 0.96, y: -10 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.96, y: -10 }}
            className="clerk-card relative w-full max-w-2xl overflow-hidden rounded-2xl"
          >
            <div className="flex items-center gap-3 border-b border-white/10 px-4 py-3.5">
              <Search className="h-4 w-4 text-zinc-400" />
              <input
                autoFocus
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search L++ docs, @pure, Keel commands, stdlib, or pages..."
                className="w-full bg-transparent text-sm text-white placeholder:text-zinc-500 focus:outline-none"
              />
              <button
                onClick={onClose}
                className="rounded-md border border-white/10 bg-white/[0.04] px-1.5 py-0.5 font-mono text-[10px] text-zinc-400 hover:text-white"
              >
                ESC
              </button>
            </div>

            <div className="max-h-[55vh] overflow-y-auto p-2">
              {filtered.length === 0 ? (
                <div className="py-10 text-center text-sm text-zinc-400">
                  No matches for &ldquo;{query}&rdquo;
                </div>
              ) : (
                filtered.map((item, idx) => (
                  <button
                    key={item.id}
                    onMouseEnter={() => setSelected(idx)}
                    onClick={() => activate(item)}
                    className={`flex w-full items-center justify-between gap-3 rounded-xl px-3.5 py-2.5 text-left transition-colors ${
                      idx === selected
                        ? "bg-[#6c47ff]/20 text-white"
                        : "text-zinc-300 hover:bg-white/[0.04]"
                    }`}
                  >
                    <div className="flex items-center gap-3 min-w-0">
                      <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-white/10 bg-[#09090b]">
                        {iconFor(item.category)}
                      </div>
                      <div className="min-w-0">
                        <div className="truncate font-mono text-xs font-semibold text-white">
                          {item.title}
                        </div>
                        <div className="truncate text-xs text-zinc-400">{item.subtitle}</div>
                      </div>
                    </div>
                    <div className="flex shrink-0 items-center gap-2">
                      <span className="rounded-md border border-white/10 bg-white/[0.04] px-2 py-0.5 font-mono text-[10px] text-zinc-400">
                        {copiedId === item.id ? "Copied!" : item.copy ? "Copy" : item.category}
                      </span>
                      <ArrowRight className="h-3.5 w-3.5 text-zinc-500" />
                    </div>
                  </button>
                ))
              )}
            </div>

            <div className="flex items-center justify-between border-t border-white/10 bg-[#09090b]/80 px-4 py-2 font-mono text-[11px] text-zinc-500">
              <span className="inline-flex items-center gap-2">
                <Code2 className="h-3.5 w-3.5 text-[#a78bfa]" />
                L++ v0.3.2 Quick Jump
              </span>
              <span className="hidden sm:inline">↑↓ navigate · ↵ select · esc close</span>
            </div>
          </motion.div>
        </div>
      )}
    </AnimatePresence>
  );
}
