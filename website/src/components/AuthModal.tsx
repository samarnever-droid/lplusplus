import { useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { X, Mail, ShieldCheck, Key, Users, Building2, Check, Copy, ArrowRight, Sparkles, AlertCircle } from "lucide-react";

function GithubIcon({ className = "h-4 w-4" }: { className?: string }) {
  return (
    <svg className={className} fill="currentColor" viewBox="0 0 24 24">
      <path fillRule="evenodd" clipRule="evenodd" d="M12 2C6.477 2 2 6.484 2 12.017c0 4.425 2.865 8.18 6.839 9.504.5.092.682-.217.682-.483 0-.237-.008-.868-.013-1.703-2.782.605-3.369-1.343-3.369-1.343-.454-1.158-1.11-1.466-1.11-1.466-.908-.62.069-.608.069-.608 1.003.07 1.53 1.032 1.53 1.032.892 1.53 2.341 1.088 2.91.832.092-.647.35-1.088.636-1.338-2.22-.253-4.555-1.113-4.555-4.951 0-1.093.39-1.988 1.029-2.688-.103-.253-.446-1.272.098-2.65 0 0 .84-.27 2.75 1.026A9.564 9.564 0 0112 6.844c.85.004 1.705.115 2.504.337 1.909-1.296 2.747-1.027 2.747-1.027.546 1.379.202 2.398.1 2.651.64.7 1.028 1.595 1.028 2.688 0 3.848-2.339 4.695-4.566 4.943.359.309.678.92.678 1.855 0 1.338-.012 2.419-.012 2.747 0 .268.18.58.688.482A10.019 10.019 0 0022 12.017C22 6.484 17.522 2 12 2z" />
    </svg>
  );
}

interface AuthModalProps {
  isOpen: boolean;
  onClose: () => void;
  initialTab?: "signin" | "signup" | "org" | "tokens";
}

interface UserState {
  id: string;
  name: string;
  email: string;
  avatarUrl: string;
  organizations: string[];
}

export default function AuthModal({ isOpen, onClose, initialTab = "signin" }: AuthModalProps) {
  const [tab, setTab] = useState<"signin" | "signup" | "org" | "tokens">(initialTab);
  const [email, setEmail] = useState("");
  const [otp, setOtp] = useState("");
  const [otpSent, setOtpSent] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [orgName, setOrgName] = useState("");
  const [createdOrgs, setCreatedOrgs] = useState<string[]>(["@community", "@personal"]);
  const [activeOrg, setActiveOrg] = useState("@personal");

  // Simulated active user or Clerk user
  const [user, setUser] = useState<UserState | null>(() => {
    const saved = localStorage.getItem("lpp_auth_user");
    return saved ? JSON.parse(saved) : null;
  });

  const [tokens, setTokens] = useState<Array<{ id: string; name: string; token: string; createdAt: string }>>(() => {
    const saved = localStorage.getItem("lpp_publisher_tokens");
    return saved ? JSON.parse(saved) : [];
  });

  const [newTokenName, setNewTokenName] = useState("Default Publisher Token");
  const [latestGeneratedToken, setLatestGeneratedToken] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const handleGithubLogin = () => {
    setLoading(true);
    // Simulate GitHub 1-click OAuth redirect/flow
    setTimeout(() => {
      const mockUser: UserState = {
        id: "user_gh_" + Math.random().toString(36).substring(2, 9),
        name: "GitHub Developer",
        email: "dev@github.com",
        avatarUrl: "https://avatars.githubusercontent.com/u/9919?v=4",
        organizations: ["@community", "@personal"],
      };
      setUser(mockUser);
      localStorage.setItem("lpp_auth_user", JSON.stringify(mockUser));
      setLoading(false);
      setTab("tokens");
    }, 800);
  };

  const handleSendOtp = (e: React.FormEvent) => {
    e.preventDefault();
    if (!email || !email.includes("@")) {
      setError("Please enter a valid developer email address");
      return;
    }
    setError("");
    setLoading(true);
    setTimeout(() => {
      setOtpSent(true);
      setLoading(false);
    }, 600);
  };

  const handleVerifyOtp = (e: React.FormEvent) => {
    e.preventDefault();
    if (otp.length < 4) {
      setError("Please enter the 6-digit OTP code sent to your email");
      return;
    }
    setError("");
    setLoading(true);
    setTimeout(() => {
      const mockUser: UserState = {
        id: "user_email_" + Math.random().toString(36).substring(2, 9),
        name: email.split("@")[0],
        email: email,
        avatarUrl: `https://api.dicebear.com/7.x/identicon/svg?seed=${email}`,
        organizations: ["@community", "@personal"],
      };
      setUser(mockUser);
      localStorage.setItem("lpp_auth_user", JSON.stringify(mockUser));
      setLoading(false);
      setTab("tokens");
    }, 600);
  };

  const handleCreateOrg = (e: React.FormEvent) => {
    e.preventDefault();
    const cleanOrg = orgName.trim().startsWith("@") ? orgName.trim().toLowerCase() : `@${orgName.trim().toLowerCase()}`;
    if (cleanOrg.length < 3) {
      setError("Organization scope must be at least 2 characters");
      return;
    }
    if (createdOrgs.includes(cleanOrg)) {
      setError("You already belong to this organization");
      return;
    }
    setError("");
    const updated = [...createdOrgs, cleanOrg];
    setCreatedOrgs(updated);
    setActiveOrg(cleanOrg);
    setOrgName("");
  };

  const handleGenerateToken = () => {
    const bytes = new Uint8Array(20);
    crypto.getRandomValues(bytes);
    const hex = Array.from(bytes)
      .map((b) => b.toString(16).padStart(2, "0"))
      .join("");
    const tokenStr = `lpp_pub_${hex}`;
    const newEntry = {
      id: "tok_" + Math.random().toString(36).substring(2, 7),
      name: newTokenName || "Publisher Token",
      token: tokenStr,
      createdAt: new Date().toLocaleDateString(),
    };
    const updated = [newEntry, ...tokens];
    setTokens(updated);
    localStorage.setItem("lpp_publisher_tokens", JSON.stringify(updated));
    setLatestGeneratedToken(tokenStr);
  };

  const handleRevokeToken = (id: string) => {
    const updated = tokens.filter((t) => t.id !== id);
    setTokens(updated);
    localStorage.setItem("lpp_publisher_tokens", JSON.stringify(updated));
    if (latestGeneratedToken && !updated.some((t) => t.token === latestGeneratedToken)) {
      setLatestGeneratedToken(null);
    }
  };

  const handleSignOut = () => {
    setUser(null);
    localStorage.removeItem("lpp_auth_user");
    setTab("signin");
  };

  const copyText = (txt: string) => {
    navigator.clipboard.writeText(txt);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
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
          className="fixed inset-0 bg-black/80 backdrop-blur-md"
        />

        <motion.div
          initial={{ scale: 0.95, opacity: 0, y: 20 }}
          animate={{ scale: 1, opacity: 1, y: 0 }}
          exit={{ scale: 0.95, opacity: 0, y: 20 }}
          className="relative w-full max-w-2xl overflow-hidden rounded-2xl border border-white/15 bg-[#0a0d12] shadow-2xl"
        >
          {/* Header */}
          <div className="flex items-center justify-between border-b border-white/10 px-6 py-4">
            <div className="flex items-center gap-3">
              <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-acid/10 border border-acid/30 text-acid">
                <ShieldCheck className="h-5 w-5" />
              </div>
              <div>
                <h2 className="text-lg font-bold text-white flex items-center gap-2">
                  L++ Developer Portal & Auth
                </h2>
                <p className="text-xs text-white/50 font-mono">
                  Clerk &bull; GitHub &bull; Email OTP &bull; Scoped CLI Tokens
                </p>
              </div>
            </div>

            <button
              onClick={onClose}
              className="grid h-8 w-8 place-items-center rounded-lg border border-white/10 text-white/60 hover:text-white hover:border-white/30"
            >
              <X className="h-4 w-4" />
            </button>
          </div>

          {/* Navigation tabs if logged in */}
          {user && (
            <div className="flex items-center justify-between border-b border-white/10 bg-white/[0.02] px-6 py-2">
              <div className="flex items-center gap-2">
                <img
                  src={user.avatarUrl}
                  alt={user.name}
                  className="h-6 w-6 rounded-full border border-white/20"
                />
                <span className="text-xs font-mono text-white/80">{user.email}</span>
              </div>

              <div className="flex items-center gap-1">
                <button
                  onClick={() => setTab("tokens")}
                  className={`rounded px-2.5 py-1 font-mono text-xs ${
                    tab === "tokens" ? "bg-acid text-ink font-semibold" : "text-white/60 hover:text-white"
                  }`}
                >
                  <Key className="inline h-3 w-3 mr-1" />
                  Tokens
                </button>
                <button
                  onClick={() => setTab("org")}
                  className={`rounded px-2.5 py-1 font-mono text-xs ${
                    tab === "org" ? "bg-acid text-ink font-semibold" : "text-white/60 hover:text-white"
                  }`}
                >
                  <Building2 className="inline h-3 w-3 mr-1" />
                  Organizations
                </button>
                <button
                  onClick={handleSignOut}
                  className="rounded px-2.5 py-1 font-mono text-xs text-rose-400 hover:bg-rose-500/10"
                >
                  Sign Out
                </button>
              </div>
            </div>
          )}

          {/* Content */}
          <div className="p-6">
            {error && (
              <div className="mb-4 flex items-center gap-2 rounded-lg border border-rose-500/30 bg-rose-500/10 p-3 text-xs text-rose-300">
                <AlertCircle className="h-4 w-4 shrink-0" />
                {error}
              </div>
            )}

            {!user ? (
              /* Sign In / Sign Up View */
              <div className="space-y-6">
                <div>
                  <h3 className="text-base font-bold text-white">Sign In to L++ Registry</h3>
                  <p className="text-xs text-white/60 mt-0.5">
                    Authenticate to publish packages, claim namespaces, or manage team organizations.
                  </p>
                </div>

                {/* 1-Click GitHub */}
                <button
                  onClick={handleGithubLogin}
                  disabled={loading}
                  className="flex w-full items-center justify-center gap-2.5 rounded-xl border border-white/20 bg-white/5 py-3 font-mono text-sm font-semibold text-white transition-all hover:bg-white/10 hover:border-white/40"
                >
                  <GithubIcon className="h-4 w-4" />
                  Continue with GitHub (1-Click)
                </button>

                <div className="relative flex items-center justify-center">
                  <div className="w-full border-t border-white/10" />
                  <span className="absolute bg-[#0a0d12] px-3 font-mono text-[11px] uppercase tracking-wider text-white/40">
                    or with Email OTP
                  </span>
                </div>

                {!otpSent ? (
                  <form onSubmit={handleSendOtp} className="space-y-3">
                    <div>
                      <label className="block text-xs font-mono text-white/70 mb-1">
                        Developer Email
                      </label>
                      <div className="relative">
                        <Mail className="absolute left-3.5 top-1/2 -translate-y-1/2 h-4 w-4 text-white/40" />
                        <input
                          type="email"
                          value={email}
                          onChange={(e) => setEmail(e.target.value)}
                          placeholder="you@domain.com"
                          className="w-full rounded-xl border border-white/10 bg-white/[0.03] pl-10 pr-4 py-2.5 font-mono text-sm text-white placeholder:text-white/30 focus:border-acid/50 focus:outline-none"
                        />
                      </div>
                    </div>

                    <button
                      type="submit"
                      disabled={loading}
                      className="flex w-full items-center justify-center gap-2 rounded-xl bg-acid py-2.5 font-mono text-sm font-bold text-ink hover:brightness-110"
                    >
                      Send One-Time Passcode (OTP)
                      <ArrowRight className="h-4 w-4" />
                    </button>
                  </form>
                ) : (
                  <form onSubmit={handleVerifyOtp} className="space-y-3">
                    <div className="rounded-lg border border-emerald-500/30 bg-emerald-500/10 p-3 text-xs text-emerald-300">
                      Passcode sent to <strong className="text-white">{email}</strong>. Enter the 6-digit OTP below.
                    </div>

                    <div>
                      <label className="block text-xs font-mono text-white/70 mb-1">
                        Enter 6-Digit OTP Code
                      </label>
                      <input
                        type="text"
                        maxLength={6}
                        value={otp}
                        onChange={(e) => setOtp(e.target.value)}
                        placeholder="123456"
                        className="w-full text-center tracking-[0.3em] font-mono text-lg rounded-xl border border-white/10 bg-white/[0.03] py-2.5 text-acid focus:border-acid/50 focus:outline-none"
                      />
                    </div>

                    <button
                      type="submit"
                      disabled={loading}
                      className="flex w-full items-center justify-center gap-2 rounded-xl bg-acid py-2.5 font-mono text-sm font-bold text-ink hover:brightness-110"
                    >
                      Verify & Sign In
                      <Check className="h-4 w-4" />
                    </button>
                  </form>
                )}
              </div>
            ) : tab === "tokens" ? (
              /* Tokens View */
              <div className="space-y-5">
                <div className="flex items-start justify-between">
                  <div>
                    <h3 className="text-base font-bold text-white flex items-center gap-2">
                      <Key className="h-4 w-4 text-acid" />
                      Publisher API Tokens
                    </h3>
                    <p className="text-xs text-white/60 mt-0.5">
                      Generate personal tokens to publish packages from the L++ CLI.
                    </p>
                  </div>
                  <button
                    onClick={handleGenerateToken}
                    className="flex items-center gap-1.5 rounded-lg bg-acid px-3 py-1.5 font-mono text-xs font-bold text-ink hover:brightness-110"
                  >
                    <Sparkles className="h-3 w-3" />
                    New Token
                  </button>
                </div>

                {latestGeneratedToken && (
                  <div className="rounded-xl border border-acid/40 bg-acid/5 p-4 space-y-2">
                    <span className="text-[11px] font-mono uppercase tracking-wider text-acid block">
                      New Token Created (Save this now — will not be shown again):
                    </span>
                    <div className="flex items-center justify-between gap-3 rounded-lg border border-white/10 bg-black/60 px-3 py-2">
                      <code className="font-mono text-xs text-acid break-all">
                        {latestGeneratedToken}
                      </code>
                      <button
                        onClick={() => copyText(latestGeneratedToken)}
                        className="flex items-center gap-1 text-xs font-mono text-white/70 hover:text-white"
                      >
                        {copied ? <Check className="h-3 w-3 text-emerald-400" /> : <Copy className="h-3 w-3" />}
                        {copied ? "Copied" : "Copy"}
                      </button>
                    </div>
                    <p className="text-xs font-mono text-white/60">
                      Quick CLI login command:
                      <code className="ml-2 text-white bg-black/40 px-2 py-0.5 rounded">
                        lpp login {latestGeneratedToken}
                      </code>
                    </p>
                  </div>
                )}

                {/* Existing tokens */}
                <div className="space-y-2">
                  <h4 className="text-xs font-mono uppercase tracking-wider text-white/40">
                    Active Publisher Tokens ({tokens.length})
                  </h4>
                  {tokens.length === 0 ? (
                    <div className="rounded-lg border border-dashed border-white/10 p-6 text-center text-xs text-white/50 font-mono">
                      No active tokens. Click "New Token" above to create one.
                    </div>
                  ) : (
                    tokens.map((t) => (
                      <div
                        key={t.id}
                        className="flex items-center justify-between rounded-lg border border-white/10 bg-white/[0.02] p-3 text-xs"
                      >
                        <div>
                          <span className="font-mono font-bold text-white block">{t.name}</span>
                          <span className="font-mono text-white/40 text-[11px]">
                            {t.token.slice(0, 12)}•••••••• &bull; Created {t.createdAt}
                          </span>
                        </div>
                        <button
                          onClick={() => handleRevokeToken(t.id)}
                          className="font-mono text-[11px] text-rose-400 hover:underline"
                        >
                          Revoke
                        </button>
                      </div>
                    ))
                  )}
                </div>
              </div>
            ) : (
              /* Organizations View */
              <div className="space-y-5">
                <div>
                  <h3 className="text-base font-bold text-white flex items-center gap-2">
                    <Building2 className="h-4 w-4 text-acid" />
                    Organizations & Scopes
                  </h3>
                  <p className="text-xs text-white/60 mt-0.5">
                    Create team organizations to publish scoped packages like <code className="text-acid">@myorg/my-pkg</code>.
                  </p>
                </div>

                <form onSubmit={handleCreateOrg} className="flex gap-2">
                  <input
                    type="text"
                    value={orgName}
                    onChange={(e) => setOrgName(e.target.value)}
                    placeholder="e.g. acme, myteam"
                    className="flex-1 rounded-xl border border-white/10 bg-white/[0.03] px-3.5 py-2 font-mono text-xs text-white focus:border-acid/50 focus:outline-none"
                  />
                  <button
                    type="submit"
                    className="rounded-xl bg-acid px-4 py-2 font-mono text-xs font-bold text-ink hover:brightness-110"
                  >
                    Create Org
                  </button>
                </form>

                <div className="space-y-2">
                  <h4 className="text-xs font-mono uppercase tracking-wider text-white/40">
                    Your Organizations ({createdOrgs.length})
                  </h4>
                  {createdOrgs.map((org) => (
                    <div
                      key={org}
                      className="flex items-center justify-between rounded-lg border border-white/10 bg-white/[0.02] p-3 text-xs"
                    >
                      <div className="flex items-center gap-2.5">
                        <div className="grid h-7 w-7 place-items-center rounded bg-white/5 font-mono font-bold text-acid text-xs">
                          {org[1]?.toUpperCase() || "@"}
                        </div>
                        <div>
                          <span className="font-mono font-bold text-white block">{org}</span>
                          <span className="font-mono text-white/40 text-[11px]">
                            {org === activeOrg ? "Active Publisher Scope" : "Member"}
                          </span>
                        </div>
                      </div>
                      {org === activeOrg ? (
                        <span className="rounded bg-acid/10 px-2 py-0.5 font-mono text-[10px] text-acid border border-acid/20">
                          Active
                        </span>
                      ) : (
                        <button
                          onClick={() => setActiveOrg(org)}
                          className="font-mono text-xs text-white/70 hover:text-acid"
                        >
                          Switch
                        </button>
                      )}
                    </div>
                  ))}
                </div>
              </div>
            )}
          </div>
        </motion.div>
      </div>
    </AnimatePresence>
  );
}
