import { useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import {
  X,
  Key,
  Terminal,
  GitBranch,
  ShieldCheck,
  Copy,
  Check,
  ExternalLink,
  Sparkles,
  LogOut,
  UserCheck,
} from "lucide-react";
import { SignInButton, SignUpButton, useAuth, useClerk, useUser } from "@clerk/react";

interface AuthModalProps {
  isOpen: boolean;
  onClose: () => void;
}

const STEPS = [
  {
    id: "registry",
    label: "1. Point Keel at your writable git registry (SSH for push)",
    command: "export KEEL_REGISTRY=git@github.com:samarnever-droid/llppregistry.git",
  },
  {
    id: "verify",
    label: "2. Check, test, and verify your package locally",
    command: "keel check && keel test && keel verify",
  },
  {
    id: "publish",
    label: "3. Pack, checksum, commit, and push the release",
    command: "keel publish",
  },
  {
    id: "consume",
    label: "4. Consumers install directly from the default registry",
    command: "keel add <pkg> && keel install && keel build",
  },
];

export default function AuthModal({ isOpen, onClose }: AuthModalProps) {
  const [tab, setTab] = useState<"clerk" | "git">("clerk");
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const { isLoaded, isSignedIn } = useAuth();
  const { user } = useUser();
  const { signOut } = useClerk();

  const copyCommand = (text: string, id: string) => {
    navigator.clipboard.writeText(text);
    setCopiedId(id);
    setTimeout(() => setCopiedId(null), 1800);
  };

  return (
    <AnimatePresence>
      {isOpen && (
        <div className="fixed inset-0 z-[100] flex items-center justify-center p-4">
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            onClick={onClose}
            className="absolute inset-0 bg-ink/85 backdrop-blur-md"
          />

          <motion.div
            initial={{ opacity: 0, scale: 0.96, y: 12 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.96, y: 12 }}
            className="relative w-full max-w-2xl max-h-[90vh] overflow-y-auto rounded-2xl border border-white/15 bg-[#0b0f17] p-6 sm:p-7 shadow-2xl"
          >
            <button
              onClick={onClose}
              className="absolute right-4 top-4 rounded-lg p-1.5 text-white/45 transition-colors hover:bg-white/10 hover:text-white"
              aria-label="Close publishing modal"
            >
              <X className="h-5 w-5" />
            </button>

            <div className="mb-5">
              <div className="mb-3 inline-flex items-center gap-2 rounded-full border border-acid/30 bg-acid/10 px-3 py-1 font-mono text-[11px] font-semibold uppercase tracking-wider text-acid">
                <Sparkles className="h-3.5 w-3.5" />
                Two Publishing Options
              </div>
              <h2 className="font-mono text-2xl font-bold text-white">
                L++ Publisher Access
              </h2>
              <p className="mt-1.5 text-sm text-white/60">
                Sign in with <span className="text-white">Clerk</span> for the feature-rich Publisher Studio, or publish directly via <span className="text-acid">Keel Git CLI</span>.
              </p>
            </div>

            {/* Option Switcher */}
            <div className="mb-5 grid grid-cols-2 gap-2 rounded-xl border border-white/10 bg-black/35 p-1">
              <button
                onClick={() => setTab("clerk")}
                className={`flex items-center justify-center gap-2 rounded-lg px-3 py-2 font-mono text-xs font-semibold transition-all ${
                  tab === "clerk"
                    ? "clerk-btn-primary"
                    : "text-white/65 hover:text-white"
                }`}
              >
                <Key className="h-3.5 w-3.5" />
                Option 1: Clerk Account
              </button>
              <button
                onClick={() => setTab("git")}
                className={`flex items-center justify-center gap-2 rounded-lg px-3 py-2 font-mono text-xs font-semibold transition-all ${
                  tab === "git"
                    ? "clerk-btn-primary"
                    : "text-white/65 hover:text-white"
                }`}
              >
                <GitBranch className="h-3.5 w-3.5" />
                Option 2: Direct Keel Git
              </button>
            </div>

            {tab === "clerk" ? (
              <div className="space-y-4">
                {!isLoaded ? (
                  <div className="rounded-xl border border-white/10 bg-black/30 p-8 text-center font-mono text-xs text-white/50">
                    Connecting to clerk.lplusplus.bond...
                  </div>
                ) : isSignedIn && user ? (
                  <div className="space-y-4 rounded-2xl border border-acid/25 bg-acid/[0.04] p-5">
                    <div className="flex flex-wrap items-center justify-between gap-4">
                      <div className="flex items-center gap-3">
                        <img
                          src={user.imageUrl}
                          alt={user.fullName || "Publisher"}
                          className="h-11 w-11 rounded-xl border border-acid/40 object-cover"
                        />
                        <div>
                          <div className="flex items-center gap-2 font-mono text-sm font-bold text-white">
                            {user.fullName || user.username || "L++ Publisher"}
                            <span className="inline-flex items-center gap-1 rounded-full border border-emerald-400/30 bg-emerald-400/10 px-2 py-0.5 text-[10px] text-emerald-300">
                              <UserCheck className="h-3 w-3" />
                              Verified
                            </span>
                          </div>
                          <div className="font-mono text-xs text-white/55">
                            {user.primaryEmailAddress?.emailAddress}
                          </div>
                        </div>
                      </div>
                      <button
                        onClick={() => signOut()}
                        className="inline-flex items-center gap-1.5 rounded-lg border border-white/15 bg-white/5 px-3 py-1.5 font-mono text-xs text-white/75 hover:border-red-400/40 hover:text-red-300"
                      >
                        <LogOut className="h-3.5 w-3.5" />
                        Sign Out
                      </button>
                    </div>
                    <p className="text-sm text-white/65">
                      Your Clerk Pro session is active on <span className="font-mono text-acid">lplusplus.bond</span>. Open the Publisher Studio to build manifests, inspect session tokens, manage MFA/passkeys, or submit packages to <span className="font-mono text-white">llppregistry</span>.
                    </p>
                    <div className="flex flex-wrap gap-3">
                      <a
                        href={`${import.meta.env.BASE_URL}account.html`}
                        className="clerk-btn-primary inline-flex items-center gap-2 rounded-xl px-4 py-2.5 font-mono text-xs font-semibold"
                      >
                        <Sparkles className="h-3.5 w-3.5" />
                        Open Publisher Studio
                        <ExternalLink className="h-3.5 w-3.5" />
                      </a>
                      <a
                        href={`${import.meta.env.BASE_URL}packages.html`}
                        className="inline-flex items-center gap-2 rounded-xl border border-white/15 bg-white/5 px-4 py-2.5 font-mono text-xs text-white/80 hover:border-acid/40 hover:text-acid"
                      >
                        Browse Registry
                      </a>
                    </div>
                  </div>
                ) : (
                  <div className="space-y-4 rounded-2xl border border-white/10 bg-black/30 p-6 text-center">
                    <div className="mx-auto flex h-12 w-12 items-center justify-center rounded-2xl border border-[#6c47ff]/40 bg-[#6c47ff]/15 text-[#a78bfa]">
                      <Key className="h-5 w-5" />
                    </div>
                    <div>
                      <h3 className="font-display text-lg font-semibold text-white">
                        Sign in with Clerk Pro
                      </h3>
                      <p className="mt-1 text-sm text-zinc-400">
                        Use GitHub, Email OTP, Passkeys, or TOTP MFA on{" "}
                        <span className="font-mono text-[#a78bfa]">clerk.lplusplus.bond</span>.
                      </p>
                    </div>
                    <div className="flex flex-wrap items-center justify-center gap-3 pt-1">
                      <SignInButton
                        mode="modal"
                        fallbackRedirectUrl={`${import.meta.env.BASE_URL}account.html`}
                      >
                        <button
                          onClick={onClose}
                          className="clerk-btn-primary inline-flex items-center gap-2 rounded-xl px-5 py-2.5 text-sm font-medium"
                        >
                          Sign In
                        </button>
                      </SignInButton>
                      <SignUpButton
                        mode="modal"
                        fallbackRedirectUrl={`${import.meta.env.BASE_URL}account.html`}
                      >
                        <button
                          onClick={onClose}
                          className="inline-flex items-center gap-2 rounded-xl border border-white/15 bg-white/[0.04] px-5 py-2.5 text-sm font-medium text-white hover:bg-white/[0.08]"
                        >
                          Create Account
                        </button>
                      </SignUpButton>
                    </div>
                  </div>
                )}
              </div>
            ) : (
              <div className="space-y-4">
                <div className="space-y-3">
                  {STEPS.map((step) => (
                    <div
                      key={step.id}
                      className="rounded-xl border border-white/10 bg-black/30 p-3.5"
                    >
                      <div className="mb-2 flex items-center justify-between gap-3">
                        <div className="flex items-center gap-2 font-mono text-xs text-white/65">
                          <Terminal className="h-3.5 w-3.5 text-acid" />
                          {step.label}
                        </div>
                        <button
                          onClick={() => copyCommand(step.command, step.id)}
                          className="inline-flex items-center gap-1 rounded-md border border-white/10 bg-white/5 px-2 py-1 font-mono text-[11px] text-white/70 hover:border-acid/40 hover:text-acid"
                        >
                          {copiedId === step.id ? (
                            <Check className="h-3 w-3 text-emerald-400" />
                          ) : (
                            <Copy className="h-3 w-3" />
                          )}
                          {copiedId === step.id ? "Copied" : "Copy"}
                        </button>
                      </div>
                      <pre className="overflow-x-auto rounded-lg border border-white/10 bg-[#05070a] px-3 py-2 font-mono text-xs text-acid">
                        {step.command}
                      </pre>
                    </div>
                  ))}
                </div>

                <div className="rounded-xl border border-emerald-400/20 bg-emerald-400/[0.04] p-4">
                  <div className="mb-1.5 flex items-center gap-2 font-mono text-xs font-bold uppercase tracking-wider text-emerald-300">
                    <ShieldCheck className="h-4 w-4" />
                    SHA-256 Verified &amp; Immutable
                  </div>
                  <p className="text-xs leading-relaxed text-white/65">
                    Every release is packed deterministically into <code className="text-white">blob/&lt;sha256&gt;</code> alongside <code className="text-white">index/&lt;sparse&gt;</code>. Consumers verify artifacts automatically with <code className="text-acid">keel verify</code>.
                  </p>
                </div>
              </div>
            )}
          </motion.div>
        </div>
      )}
    </AnimatePresence>
  );
}
