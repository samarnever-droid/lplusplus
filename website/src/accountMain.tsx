import React, { useState } from "react";
import ReactDOM from "react-dom/client";
import {
  ArrowLeft,
  Check,
  Copy,
  ExternalLink,
  GitBranch,
  Key,
  Layers,
  Lock,
  Package,
  RefreshCw,
  ShieldCheck,
  Terminal,
  UploadCloud,
} from "lucide-react";
import Footer from "./components/Footer";
import "./index.css";

const COMMANDS = [
  {
    id: "registry",
    label: "Choose the git registry remote",
    command: "export KEEL_REGISTRY=git@github.com:samarnever-droid/llppregistry.git",
  },
  {
    id: "init",
    label: "Create or inspect the manifest",
    command: "keel init  # creates Keel.toml when starting a new package",
  },
  {
    id: "verify",
    label: "Verify locally before publish",
    command: "keel check && keel test && keel verify",
  },
  {
    id: "publish",
    label: "Publish as a git commit + push",
    command: "keel publish",
  },
  {
    id: "consumer",
    label: "Consumer update path",
    command: "keel fetch && keel update && keel build",
  },
];

const STATUS = [
  { label: "Domain", value: "lplusplus.bond", tone: "text-acid" },
  { label: "Registry mirror", value: "registry.lplusplus.bond", tone: "text-emerald-300" },
  { label: "Authority", value: "git index + blob", tone: "text-sky-300" },
  { label: "Auth", value: "git push rights", tone: "text-violet-300" },
];

const FLOW = [
  {
    icon: Package,
    title: "Manifest",
    text: "Keel.toml declares package name, version, edition, license, features, targets, and dependencies.",
  },
  {
    icon: UploadCloud,
    title: "Artifact",
    text: "Keel packs the source and stores it by SHA-256, so downloads are tamper-evident and reproducible.",
  },
  {
    icon: GitBranch,
    title: "Registry commit",
    text: "index/<sparse-path> and blob/<sha256> are committed together. Duplicate versions are immutable.",
  },
  {
    icon: RefreshCw,
    title: "Mirrors update",
    text: "GitHub Pages and the Cloudflare Worker expose read-only index/search/status endpoints for the website.",
  },
];

function CopyCommand({ id, label, command, copied, onCopy }: { id: string; label: string; command: string; copied: string | null; onCopy: (text: string, id: string) => void }) {
  return (
    <div className="rounded-2xl border border-white/10 bg-black/30 p-4">
      <div className="mb-2 flex items-center justify-between gap-3">
        <div className="flex items-center gap-2 font-mono text-xs font-bold uppercase tracking-wider text-white/55">
          <Terminal className="h-3.5 w-3.5 text-acid" />
          {label}
        </div>
        <button
          onClick={() => onCopy(command, id)}
          className="inline-flex items-center gap-1.5 rounded-lg border border-white/10 bg-white/5 px-2.5 py-1 font-mono text-[11px] text-white/70 hover:border-acid/40 hover:text-acid"
        >
          {copied === id ? <Check className="h-3 w-3 text-emerald-400" /> : <Copy className="h-3 w-3" />}
          {copied === id ? "Copied" : "Copy"}
        </button>
      </div>
      <pre className="overflow-x-auto rounded-xl border border-white/10 bg-[#05070a] p-3 font-mono text-xs text-acid">
        {command}
      </pre>
    </div>
  );
}

function AccountApp() {
  const [copied, setCopied] = useState<string | null>(null);

  const copyText = (text: string, id: string) => {
    navigator.clipboard.writeText(text);
    setCopied(id);
    setTimeout(() => setCopied(null), 1800);
  };

  return (
    <div className="min-h-screen bg-[#07090d] text-white flex flex-col font-sans antialiased">
      <header className="sticky top-0 z-40 border-b border-white/10 bg-[#07090d]/90 backdrop-blur-xl">
        <div className="mx-auto flex h-16 max-w-7xl items-center justify-between px-5 md:px-8">
          <a href="/" className="flex items-center gap-2.5 text-white/80 hover:text-white">
            <ArrowLeft className="h-4 w-4" />
            <span className="font-mono text-xs">Back to Main</span>
          </a>
          <div className="flex items-center gap-2 rounded-full border border-acid/25 bg-acid/10 px-3 py-1 font-mono text-xs text-acid">
            <ShieldCheck className="h-3.5 w-3.5" />
            Git-backed publisher guide
          </div>
        </div>
      </header>

      <main className="mx-auto max-w-7xl w-full flex-1 px-5 md:px-8 py-10 space-y-10">
        <section className="overflow-hidden rounded-3xl border border-white/10 bg-gradient-to-br from-white/[0.06] via-white/[0.025] to-acid/[0.04] p-6 md:p-10 shadow-2xl">
          <div className="grid gap-8 lg:grid-cols-[1.2fr_0.8fr] lg:items-center">
            <div className="space-y-6">
              <div className="inline-flex items-center gap-2 rounded-full border border-emerald-400/30 bg-emerald-400/10 px-3 py-1 font-mono text-xs text-emerald-300">
                <Lock className="h-3.5 w-3.5" />
                Clerk removed — git push rights are the auth layer
              </div>
              <div>
                <h1 className="max-w-3xl font-mono text-3xl font-black leading-tight tracking-tight text-white sm:text-5xl">
                  Publish L++ packages through a durable <span className="text-acid">git registry</span>
                </h1>
                <p className="mt-4 max-w-2xl text-sm leading-7 text-white/65 sm:text-base">
                  The old browser token flow was not durable: Worker memory cannot be the package registry. The cleaned design uses Keel to pack,
                  checksum, commit, and push package releases into a git repository. The website and Cloudflare Worker are fast read-only mirrors.
                </p>
              </div>
              <div className="flex flex-wrap gap-3">
                <a
                  href="/packages.html"
                  className="inline-flex items-center gap-2 rounded-xl bg-acid px-4 py-2.5 font-mono text-xs font-bold text-ink hover:brightness-110"
                >
                  Browse packages
                  <Package className="h-3.5 w-3.5" />
                </a>
                <a
                  href="https://registry.lplusplus.bond/health"
                  className="inline-flex items-center gap-2 rounded-xl border border-white/15 bg-white/5 px-4 py-2.5 font-mono text-xs text-white/80 hover:border-acid/40 hover:text-acid"
                >
                  Worker health
                  <ExternalLink className="h-3.5 w-3.5" />
                </a>
              </div>
            </div>

            <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-1">
              {STATUS.map((item) => (
                <div key={item.label} className="rounded-2xl border border-white/10 bg-black/25 p-4">
                  <div className="font-mono text-[11px] uppercase tracking-wider text-white/40">{item.label}</div>
                  <div className={`mt-1 font-mono text-sm font-bold ${item.tone}`}>{item.value}</div>
                </div>
              ))}
            </div>
          </div>
        </section>

        <section className="grid gap-4 md:grid-cols-4">
          {FLOW.map((item) => {
            const Icon = item.icon;
            return (
              <div key={item.title} className="rounded-2xl border border-white/10 bg-white/[0.03] p-5">
                <div className="mb-4 flex h-10 w-10 items-center justify-center rounded-xl border border-acid/25 bg-acid/10 text-acid">
                  <Icon className="h-5 w-5" />
                </div>
                <h2 className="font-mono text-base font-bold text-white">{item.title}</h2>
                <p className="mt-2 text-sm leading-relaxed text-white/60">{item.text}</p>
              </div>
            );
          })}
        </section>

        <section className="grid gap-8 lg:grid-cols-[0.95fr_1.05fr]">
          <div className="rounded-3xl border border-white/10 bg-white/[0.03] p-6">
            <div className="mb-5 flex items-center gap-3">
              <div className="flex h-10 w-10 items-center justify-center rounded-xl border border-acid/25 bg-acid/10 text-acid">
                <Key className="h-5 w-5" />
              </div>
              <div>
                <h2 className="font-mono text-lg font-bold text-white">What changed</h2>
                <p className="font-mono text-xs text-white/45">No SaaS auth dependency, no fake tokens.</p>
              </div>
            </div>
            <div className="space-y-4 text-sm leading-7 text-white/65">
              <p>
                Clerk and Supabase were only partially wired. The website could generate local-looking tokens, but the Worker stored token and publish state in memory.
                That means a restart could forget publishers and packages.
              </p>
              <p>
                The fixed model removes browser auth from the critical path. Package mutation is now the same security model developers already understand:
                push access to a git repository, optionally protected by GitHub branch rules or pull-request review.
              </p>
              <div className="rounded-2xl border border-emerald-400/20 bg-emerald-400/[0.04] p-4 text-emerald-100/80">
                <ShieldCheck className="mb-2 h-5 w-5 text-emerald-300" />
                Existing locked builds remain reproducible because Keel verifies every artifact by checksum from Keel.lock.
              </div>
            </div>
          </div>

          <div className="space-y-3">
            {COMMANDS.map((item) => (
              <CopyCommand key={item.id} {...item} copied={copied} onCopy={copyText} />
            ))}
          </div>
        </section>

        <section className="rounded-3xl border border-white/10 bg-white/[0.03] p-6 md:p-8">
          <div className="mb-6 flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-xl border border-acid/25 bg-acid/10 text-acid">
              <Layers className="h-5 w-5" />
            </div>
            <div>
              <h2 className="font-mono text-lg font-bold text-white">Runtime surfaces and env</h2>
              <p className="font-mono text-xs text-white/45">Only the values below are needed for the public mirror.</p>
            </div>
          </div>
          <div className="grid gap-3 md:grid-cols-2">
            <div className="rounded-2xl border border-white/10 bg-black/25 p-4">
              <div className="font-mono text-xs text-white/40">Cloudflare vars</div>
              <pre className="mt-3 overflow-x-auto rounded-xl bg-[#05070a] p-3 font-mono text-xs leading-6 text-acid">{`DOMAIN=lplusplus.bond
REGISTRY_URL=https://registry.lplusplus.bond
STATIC_INDEX_URL=https://lplusplus.bond/registry/index.json
GITHUB_REPO=samarnever-droid/llppregistry`}</pre>
            </div>
            <div className="rounded-2xl border border-white/10 bg-black/25 p-4">
              <div className="font-mono text-xs text-white/40">Optional secret</div>
              <pre className="mt-3 overflow-x-auto rounded-xl bg-[#05070a] p-3 font-mono text-xs leading-6 text-acid">wrangler secret put GITHUB_TOKEN</pre>
              <p className="mt-3 text-sm text-white/55">Used only for GitHub API rate limits on download statistics. It is not a publish credential.</p>
            </div>
          </div>
        </section>
      </main>

      <Footer />
    </div>
  );
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <AccountApp />
  </React.StrictMode>
);
