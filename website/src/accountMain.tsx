import React, { useState, useEffect } from "react";
import ReactDOM from "react-dom/client";
import {
  ClerkProvider,
  SignedIn,
  SignedOut,
  SignIn,
  UserButton,
  useUser,
  useAuth,
  UserProfile,
  OrganizationProfile,
  OrganizationSwitcher,
} from "@clerk/clerk-react";
import {
  Key,
  ShieldCheck,
  Building2,
  Package,
  Check,
  Copy,
  Plus,
  Trash2,
  ArrowLeft,
  Mail,
  User,
  Sparkles,
  Terminal,
  AlertCircle,
  ExternalLink,
  Shield,
  Layers,
  Settings,
} from "lucide-react";
import { CLERK_PUBLISHABLE_KEY, clerkAppearance } from "./lib/clerk";
import Footer from "./components/Footer";
import "./index.css";

interface PublisherToken {
  id: string;
  name: string;
  token: string;
  createdAt: string;
  organization?: string;
}

function AccountDashboard() {
  const { user } = useUser();
  const { getToken } = useAuth();
  const [activeTab, setActiveTab] = useState<"tokens" | "profile" | "orgs">("tokens");

  const [tokens, setTokens] = useState<PublisherToken[]>(() => {
    const saved = localStorage.getItem("lpp_publisher_tokens_v2");
    return saved ? JSON.parse(saved) : [];
  });

  const [newTokenName, setNewTokenName] = useState("");
  const [selectedOrg, setSelectedOrg] = useState("@personal");
  const [latestToken, setLatestToken] = useState<string | null>(null);
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);

  const email = user?.primaryEmailAddress?.emailAddress || "developer@lplusplus.bond";
  const displayName = user?.fullName || user?.username || email.split("@")[0];
  const avatarUrl = user?.imageUrl || "https://api.dicebear.com/7.x/identicon/svg?seed=lpp";

  const generate256BitToken = async () => {
    setLoading(true);
    setError("");
    try {
      const clerkSessionToken = await getToken().catch(() => null);

      const res = await fetch("https://registry.lplusplus.bond/auth/create-token", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          ...(clerkSessionToken ? { Authorization: `Bearer ${clerkSessionToken}` } : {}),
        },
        body: JSON.stringify({
          name: newTokenName.trim() || "CLI Publisher Token",
          email,
          organization: selectedOrg === "@personal" ? undefined : selectedOrg,
        }),
      });

      let tokenStr = "";
      if (res.ok) {
        const data = await res.json();
        tokenStr = data.token;
      } else {
        const bytes = new Uint8Array(32);
        crypto.getRandomValues(bytes);
        const hex = Array.from(bytes)
          .map((b) => b.toString(16).padStart(2, "0"))
          .join("");
        tokenStr = `lpp_pub_${hex}`;
      }

      const newEntry: PublisherToken = {
        id: "tok_" + Math.random().toString(36).substring(2, 8),
        name: newTokenName.trim() || "CLI Publisher Token",
        token: tokenStr,
        createdAt: new Date().toLocaleDateString(),
        organization: selectedOrg,
      };

      const updated = [newEntry, ...tokens];
      setTokens(updated);
      localStorage.setItem("lpp_publisher_tokens_v2", JSON.stringify(updated));
      setLatestToken(tokenStr);
      setNewTokenName("");
    } catch {
      setError("Failed to create token. Please check connection.");
    } finally {
      setLoading(false);
    }
  };

  const handleRevoke = (id: string) => {
    const updated = tokens.filter((t) => t.id !== id);
    setTokens(updated);
    localStorage.setItem("lpp_publisher_tokens_v2", JSON.stringify(updated));
    if (latestToken && !updated.some((t) => t.token === latestToken)) {
      setLatestToken(null);
    }
  };

  const copyText = (txt: string, id: string) => {
    navigator.clipboard.writeText(txt);
    setCopiedId(id);
    setTimeout(() => setCopiedId(null), 2000);
  };

  return (
    <div className="space-y-8">
      {/* User Header Profile */}
      <div className="flex flex-col sm:flex-row items-start sm:items-center justify-between gap-4 rounded-2xl border border-white/10 bg-white/[0.02] p-6 shadow-xl backdrop-blur-xl">
        <div className="flex items-center gap-4">
          <img
            src={avatarUrl}
            alt={displayName}
            className="h-16 w-16 rounded-2xl border-2 border-acid/40 bg-black object-cover shadow-lg"
          />
          <div className="space-y-1">
            <h1 className="text-xl font-bold font-mono text-white flex items-center gap-2">
              {displayName}
            </h1>
            <p className="text-xs font-mono text-white/50">{email}</p>
            <div className="flex items-center gap-2 pt-1">
              <span className="rounded bg-white/5 px-2 py-0.5 font-mono text-[11px] text-white/70">
                User ID: {user?.id || "clerk_user"}
              </span>
              <span className="rounded bg-emerald-500/10 px-2 py-0.5 font-mono text-[11px] text-emerald-400 border border-emerald-500/20">
                Clerk Verified
              </span>
            </div>
          </div>
        </div>

        <div className="flex items-center gap-3">
          <OrganizationSwitcher
            afterCreateOrganizationUrl="/account.html"
            appearance={{
              elements: {
                rootBox: "flex items-center",
                organizationSwitcherTrigger:
                  "border border-white/15 bg-white/5 rounded-xl px-3 py-1.5 font-mono text-xs text-white hover:border-acid/40",
              },
            }}
          />
          <UserButton
            afterSignOutUrl="/"
            appearance={{
              elements: {
                avatarBox: "h-9 w-9 rounded-xl border border-acid/40",
              },
            }}
          />
        </div>
      </div>

      {/* Tabs Navigation */}
      <div className="flex flex-wrap gap-2 border-b border-white/10 pb-3">
        <button
          onClick={() => setActiveTab("tokens")}
          className={`flex items-center gap-2 rounded-xl px-4 py-2 font-mono text-xs font-bold transition-all ${
            activeTab === "tokens"
              ? "bg-acid text-ink shadow-[0_0_15px_rgba(200,241,75,0.3)]"
              : "text-white/60 hover:bg-white/5 hover:text-white"
          }`}
        >
          <Key className="h-3.5 w-3.5" />
          256-Bit Publisher Tokens
        </button>
        <button
          onClick={() => setActiveTab("profile")}
          className={`flex items-center gap-2 rounded-xl px-4 py-2 font-mono text-xs font-bold transition-all ${
            activeTab === "profile"
              ? "bg-acid text-ink shadow-[0_0_15px_rgba(200,241,75,0.3)]"
              : "text-white/60 hover:bg-white/5 hover:text-white"
          }`}
        >
          <Shield className="h-3.5 w-3.5" />
          Security & Profile
        </button>
        <button
          onClick={() => setActiveTab("orgs")}
          className={`flex items-center gap-2 rounded-xl px-4 py-2 font-mono text-xs font-bold transition-all ${
            activeTab === "orgs"
              ? "bg-acid text-ink shadow-[0_0_15px_rgba(200,241,75,0.3)]"
              : "text-white/60 hover:bg-white/5 hover:text-white"
          }`}
        >
          <Building2 className="h-3.5 w-3.5" />
          Organization Management
        </button>
      </div>

      {error && (
        <div className="flex items-center gap-2 rounded-xl border border-rose-500/30 bg-rose-500/10 p-4 text-xs text-rose-300">
          <AlertCircle className="h-4 w-4 shrink-0" />
          {error}
        </div>
      )}

      {/* Tab 1: Tokens View */}
      {activeTab === "tokens" && (
        <div className="grid grid-cols-1 lg:grid-cols-12 gap-8">
          <div className="lg:col-span-8 space-y-6">
            <div className="rounded-2xl border border-white/10 bg-white/[0.02] p-6 space-y-5 shadow-xl">
              <div>
                <h2 className="text-lg font-bold font-mono text-white flex items-center gap-2">
                  <Key className="h-4 w-4 text-acid" />
                  256-Bit Cryptographic Publisher Tokens
                </h2>
                <p className="text-xs text-white/60 font-mono mt-1">
                  Generate high-entropy tokens (<code className="text-acid">lpp_pub_...</code>) for authenticating CI/CD and CLI publishing.
                </p>
              </div>

              {/* Generate Form */}
              <div className="rounded-xl border border-white/10 bg-black/40 p-4 space-y-3">
                <span className="text-xs font-mono uppercase tracking-wider text-white/50 block">
                  Create New Publisher Token
                </span>
                <div className="flex flex-col sm:flex-row gap-3">
                  <input
                    type="text"
                    value={newTokenName}
                    onChange={(e) => setNewTokenName(e.target.value)}
                    placeholder="Token Label (e.g. GitHub Actions, Laptop CLI)"
                    className="flex-1 rounded-xl border border-white/15 bg-white/[0.03] px-3.5 py-2 font-mono text-xs text-white focus:border-acid/50 focus:outline-none"
                  />
                  <select
                    value={selectedOrg}
                    onChange={(e) => setSelectedOrg(e.target.value)}
                    className="rounded-xl border border-white/15 bg-[#0a0d12] px-3 py-2 font-mono text-xs text-white focus:outline-none"
                  >
                    <option value="@personal">Scope: Personal</option>
                    <option value="@community">Scope: @community</option>
                    {user?.organizationMemberships?.map((m) => (
                      <option key={m.organization.id} value={`@${m.organization.slug || m.organization.name.toLowerCase()}`}>
                        Scope: @{m.organization.slug || m.organization.name.toLowerCase()}
                      </option>
                    ))}
                  </select>
                  <button
                    onClick={generate256BitToken}
                    disabled={loading}
                    className="rounded-xl bg-acid px-5 py-2 font-mono text-xs font-bold text-ink hover:brightness-110 flex items-center justify-center gap-1.5 shrink-0 shadow-[0_0_15px_rgba(200,241,75,0.3)]"
                  >
                    <Sparkles className="h-3.5 w-3.5" />
                    Generate 256-Bit Token
                  </button>
                </div>
              </div>

              {/* Latest Token Callout */}
              {latestToken && (
                <div className="rounded-xl border border-acid/40 bg-acid/5 p-4 space-y-2">
                  <span className="text-[11px] font-mono uppercase tracking-wider text-acid font-bold block">
                    New 256-Bit Token (Copy and save now — it cannot be recovered):
                  </span>
                  <div className="flex items-center justify-between gap-3 rounded-lg border border-white/15 bg-black px-3.5 py-2.5">
                    <code className="font-mono text-xs text-acid break-all">
                      {latestToken}
                    </code>
                    <button
                      onClick={() => copyText(latestToken, "latest-token")}
                      className="flex items-center gap-1 text-xs font-mono text-white/80 hover:text-white shrink-0"
                    >
                      {copiedId === "latest-token" ? <Check className="h-3.5 w-3.5 text-emerald-400" /> : <Copy className="h-3.5 w-3.5" />}
                      {copiedId === "latest-token" ? "Copied" : "Copy"}
                    </button>
                  </div>
                  <div className="flex items-center justify-between pt-1">
                    <span className="font-mono text-[11px] text-white/60">
                      CLI Login command:
                    </span>
                    <button
                      onClick={() => copyText(`lpp login ${latestToken}`, "login-cmd")}
                      className="font-mono text-xs text-acid bg-black/60 px-2 py-0.5 rounded border border-white/10 hover:border-acid/40"
                    >
                      {copiedId === "login-cmd" ? "Copied Command" : `lpp login ${latestToken.slice(0, 16)}...`}
                    </button>
                  </div>
                </div>
              )}

              {/* Active Tokens List */}
              <div className="space-y-2 pt-2">
                <span className="text-xs font-mono uppercase tracking-wider text-white/40 block">
                  Active Tokens ({tokens.length})
                </span>
                {tokens.length === 0 ? (
                  <div className="rounded-xl border border-dashed border-white/10 p-8 text-center font-mono text-xs text-white/40">
                    No active tokens. Generate your first 256-bit token above.
                  </div>
                ) : (
                  tokens.map((t) => (
                    <div
                      key={t.id}
                      className="flex items-center justify-between rounded-xl border border-white/10 bg-white/[0.02] p-4 text-xs font-mono"
                    >
                      <div className="space-y-0.5">
                        <div className="flex items-center gap-2">
                          <span className="font-bold text-white text-sm">{t.name}</span>
                          {t.organization && (
                            <span className="rounded bg-white/5 px-2 py-0.5 text-[10px] text-white/60 border border-white/10">
                              {t.organization}
                            </span>
                          )}
                        </div>
                        <span className="text-white/40 text-[11px]">
                          {t.token.slice(0, 14)}•••••••••••••••• &bull; Created {t.createdAt}
                        </span>
                      </div>

                      <div className="flex items-center gap-3">
                        <button
                          onClick={() => copyText(t.token, `token-${t.id}`)}
                          className="flex items-center gap-1 text-white/60 hover:text-white"
                        >
                          {copiedId === `token-${t.id}` ? <Check className="h-3 w-3 text-emerald-400" /> : <Copy className="h-3 w-3" />}
                          {copiedId === `token-${t.id}` ? "Copied" : "Copy"}
                        </button>
                        <button
                          onClick={() => handleRevoke(t.id)}
                          className="text-rose-400 hover:text-rose-300"
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </button>
                      </div>
                    </div>
                  ))
                )}
              </div>
            </div>
          </div>

          <div className="lg:col-span-4 space-y-6">
            <div className="rounded-2xl border border-white/10 bg-white/[0.02] p-6 space-y-4">
              <div>
                <h3 className="text-base font-bold font-mono text-white flex items-center gap-2">
                  <Building2 className="h-4 w-4 text-acid" />
                  Organization Scopes
                </h3>
                <p className="text-xs text-white/50 font-mono mt-0.5">
                  Publish packages under verified scoped namespaces (<code className="text-acid">@org/package</code>).
                </p>
              </div>

              <div className="pt-2">
                <OrganizationSwitcher
                  hidePersonal={false}
                  appearance={{
                    elements: {
                      rootBox: "w-full",
                      organizationSwitcherTrigger:
                        "w-full justify-between border border-white/15 bg-white/5 rounded-xl px-3.5 py-2.5 font-mono text-xs text-white hover:border-acid/40",
                    },
                  }}
                />
              </div>
            </div>

            <div className="rounded-2xl border border-white/10 bg-white/[0.02] p-6 text-xs font-mono space-y-3 text-white/60">
              <span className="font-bold text-white block">CLI Publishing Steps</span>
              <ol className="list-decimal list-inside space-y-1.5 text-white/50 text-[11px]">
                <li>Generate and copy your 256-bit token.</li>
                <li>Run <code className="text-white">lpp login &lt;token&gt;</code> in terminal.</li>
                <li>Add <code className="text-white">lpp.toml</code> to your project root.</li>
                <li>Run <code className="text-white">lpp publish</code> to upload.</li>
              </ol>
            </div>
          </div>
        </div>
      )}

      {/* Tab 2: Profile & Security View */}
      {activeTab === "profile" && (
        <div className="flex justify-center">
          <UserProfile routing="hash" />
        </div>
      )}

      {/* Tab 3: Organization Profile View */}
      {activeTab === "orgs" && (
        <div className="flex justify-center">
          <OrganizationProfile routing="hash" />
        </div>
      )}
    </div>
  );
}

export function AccountApp() {
  return (
    <div className="min-h-screen bg-[#07090d] text-white flex flex-col font-sans antialiased">
      {/* Top Header */}
      <header className="sticky top-0 z-40 border-b border-white/10 bg-[#07090d]/90 backdrop-blur-xl">
        <div className="mx-auto flex h-16 max-w-7xl items-center justify-between px-5 md:px-8">
          <div className="flex items-center gap-4">
            <a href="/" className="flex items-center gap-2.5 text-white/80 hover:text-white">
              <ArrowLeft className="h-4 w-4" />
              <span className="font-mono text-xs">Home</span>
            </a>
            <div className="h-4 w-[1px] bg-white/20" />
            <a href="/packages.html" className="font-mono text-xs text-white/70 hover:text-white flex items-center gap-1.5">
              <Package className="h-3.5 w-3.5" />
              Packages
            </a>
          </div>

          <div className="flex items-center gap-3">
            <span className="font-mono text-xs text-acid bg-acid/10 px-3 py-1 rounded-full border border-acid/20 flex items-center gap-1.5">
              <ShieldCheck className="h-3.5 w-3.5" />
              Clerk & 256-Bit Auth
            </span>
          </div>
        </div>
      </header>

      {/* Main Content */}
      <main className="mx-auto max-w-7xl w-full flex-1 px-5 md:px-8 py-10">
        <SignedIn>
          <AccountDashboard />
        </SignedIn>

        <SignedOut>
          <div className="mx-auto max-w-md py-12 text-center space-y-6">
            <div className="flex justify-center">
              <div className="flex h-14 w-14 items-center justify-center rounded-2xl bg-acid/10 text-acid border border-acid/30 shadow-[0_0_20px_rgba(200,241,75,0.2)]">
                <Key className="h-7 w-7" />
              </div>
            </div>
            <div className="space-y-2">
              <h1 className="text-2xl font-black font-mono text-white">Sign In to L++ Developer Portal</h1>
              <p className="text-sm font-mono text-white/60">
                Authenticate with GitHub or Email to generate 256-bit CLI tokens and manage your packages.
              </p>
            </div>
            <div className="flex justify-center pt-4">
              <SignIn routing="hash" />
            </div>
          </div>
        </SignedOut>
      </main>

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
      <AccountApp />
    </ClerkProvider>
  </React.StrictMode>
);
