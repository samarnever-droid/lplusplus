import { useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import {
  X,
  ShieldCheck,
  Copy,
  Check,
  Terminal,
  GitBranch,
  Package,
  UploadCloud,
  RefreshCw,
  Lock,
  ExternalLink,
} from "lucide-react";

interface AuthModalProps {
  isOpen: boolean;
  onClose: () => void;
  initialTab?: "signin" | "signup" | "org" | "tokens";
}

const COMMANDS = [
  {
    id: "login",
    title: "Point Keel at the git registry",
    command: "export KEEL_REGISTRY=git@github.com:samarnever-droid/llppregistry.git",
  },
  {
    id: "check",
    title: "Verify before publishing",
    command: "keel check && keel test && keel verify",
  },
  {
    id: "publish",
    title: "Publish as one atomic git commit",
    command: "keel publish",
  },
  {
    id: "update",
    title: "Consumers update from git",
    command: "keel fetch && keel update",
  },
];

const FLOW = [
  {
    icon: Package,
    title: "Keel.toml manifest",
    text: "Name, version, license, features, target list, and dependencies live in one package manifest.",
  },
  {
    icon: UploadCloud,
    title: "Content-addressed artifact",
    text: "Keel packs the source, computes SHA-256, writes blob/<sha256>, and refuses mutable duplicate versions.",
  },
  {
    icon: GitBranch,
    title: "Git is the authority",
    text: "The registry index and blob land in a normal git commit. Push rights replace browser tokens.",
  },
  {
    icon: RefreshCw,
    title: "Static mirrors refresh",
    text: "GitHub Pages and registry.lplusplus.bond mirror the index for browsing and search. Keel can still work offline from the git clone.",
  },
];

export default function AuthModal({ isOpen, onClose }: AuthModalProps) {
  const [copied, setCopied] = useState<string | null>(null);

  const copyText = (text: string, id: string) => {
    navigator.clipboard.writeText(text);
    setCopied(id);
    setTimeout(() => setCopied(null), 1800);
  };

  if (!isOpen) return null;

  return (
    <AnimatePresence>
      <div className="fixed inset-0 z-50 flex items-center justify-center p-4 sm:p-6 md:p-10">
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
          className="relative flex max-h-[90vh] w-full max-w-4xl flex-col overflow-hidden rounded-2xl border border-white/15 bg-[#090c10] shadow-2xl"
        >
          <div className="flex items-center justify-between border-b border-white/10 px-6 py-4">
            <div className="flex items-center gap-3">
              <div className="flex h-10 w-10 items-center justify-center rounded-xl border border-acid/30 bg-acid/10 text-acid">
                <ShieldCheck className="h-5 w-5" />
              </div>
              <div>
                <h2 className="font-mono text-lg font-bold text-white">Publish to L++ Registry</h2>
                <p className="font-mono text-xs text-white/50">No Clerk. No fake browser tokens. Git-backed, checksummed, reproducible.</p>
              </div>
            </div>
            <button
              onClick={onClose}
              className="grid h-8 w-8 place-items-center rounded-lg border border-white/10 text-white/60 hover:border-white/30 hover:text-white"
            >
              <X className="h-4 w-4" />
            </button>
          </div>

          <div className="overflow-y-auto p-6 space-y-6">
            <div className="grid gap-4 md:grid-cols-4">
              {FLOW.map((item) => {
                const Icon = item.icon;
                return (
                  <div key={item.title} className="rounded-2xl border border-white/10 bg-white/[0.03] p-4">
                    <div className="mb-3 flex h-9 w-9 items-center justify-center rounded-xl border border-acid/25 bg-acid/10 text-acid">
                      <Icon className="h-4 w-4" />
                    </div>
                    <h3 className="font-mono text-sm font-bold text-white">{item.title}</h3>
                    <p className="mt-2 text-xs leading-relaxed text-white/55">{item.text}</p>
                  </div>
                );
              })}
            </div>

            <div className="rounded-2xl border border-emerald-400/20 bg-emerald-400/[0.04] p-5">
              <div className="flex items-start gap-3">
                <Lock className="mt-0.5 h-5 w-5 text-emerald-400" />
                <div className="space-y-2">
                  <h3 className="font-mono text-sm font-bold text-emerald-300">Authentication is git access</h3>
                  <p className="text-sm leading-relaxed text-white/65">
                    Publisher authority now comes from SSH keys, deploy keys, GitHub PATs, or reviewed pull requests on the registry repo.
                    The Cloudflare Worker is read-only, so a compromised browser session cannot mutate packages.
                  </p>
                </div>
              </div>
            </div>

            <div className="grid gap-3">
              {COMMANDS.map((item) => (
                <div key={item.id} className="rounded-2xl border border-white/10 bg-black/30 p-4">
                  <div className="mb-2 flex items-center justify-between gap-3">
                    <div className="flex items-center gap-2 font-mono text-xs font-bold uppercase tracking-wider text-white/60">
                      <Terminal className="h-3.5 w-3.5 text-acid" />
                      {item.title}
                    </div>
                    <button
                      onClick={() => copyText(item.command, item.id)}
                      className="inline-flex items-center gap-1.5 rounded-lg border border-white/10 bg-white/5 px-2.5 py-1 font-mono text-[11px] text-white/70 hover:border-acid/40 hover:text-acid"
                    >
                      {copied === item.id ? <Check className="h-3 w-3 text-emerald-400" /> : <Copy className="h-3 w-3" />}
                      {copied === item.id ? "Copied" : "Copy"}
                    </button>
                  </div>
                  <pre className="overflow-x-auto rounded-xl border border-white/10 bg-[#05070a] p-3 font-mono text-xs text-acid">
                    {item.command}
                  </pre>
                </div>
              ))}
            </div>

            <div className="grid gap-4 md:grid-cols-2">
              <div className="rounded-2xl border border-white/10 bg-white/[0.03] p-5">
                <h3 className="mb-3 font-mono text-sm font-bold text-white">What was removed</h3>
                <ul className="space-y-2 text-sm text-white/60">
                  <li>• Clerk publishable key and Clerk React UI.</li>
                  <li>• Supabase placeholders that were not used by the Worker.</li>
                  <li>• In-memory Worker publisher tokens.</li>
                  <li>• In-memory Worker publish route that lost data on restart.</li>
                </ul>
              </div>
              <div className="rounded-2xl border border-white/10 bg-white/[0.03] p-5">
                <h3 className="mb-3 font-mono text-sm font-bold text-white">Live surfaces</h3>
                <ul className="space-y-2 text-sm text-white/60">
                  <li>• Website: <code className="text-white">lplusplus.bond</code></li>
                  <li>• Registry mirror: <code className="text-white">registry.lplusplus.bond</code></li>
                  <li>• Static index: <code className="text-white">/registry/index.json</code></li>
                  <li>• Canonical writes: <code className="text-white">keel publish</code></li>
                </ul>
              </div>
            </div>

            <div className="flex flex-col gap-3 rounded-2xl border border-acid/20 bg-acid/[0.04] p-5 sm:flex-row sm:items-center sm:justify-between">
              <div>
                <h3 className="font-mono text-sm font-bold text-acid">Need the full package docs?</h3>
                <p className="mt-1 text-sm text-white/60">The account page is now a permanent publisher guide with the same git-backed flow.</p>
              </div>
              <a
                href="/account.html"
                className="inline-flex items-center justify-center gap-2 rounded-xl bg-acid px-4 py-2 font-mono text-xs font-bold text-ink hover:brightness-110"
              >
                Open Publisher Guide
                <ExternalLink className="h-3.5 w-3.5" />
              </a>
            </div>
          </div>
        </motion.div>
      </div>
    </AnimatePresence>
  );
}
