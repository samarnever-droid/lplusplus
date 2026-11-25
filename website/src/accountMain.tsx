import React, { useState } from "react";
import ReactDOM from "react-dom/client";
import {
  Activity,
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
  Shield,
  ShieldCheck,
  Sparkles,
  Terminal,
  UploadCloud,
  UserCheck,
  Users,
} from "lucide-react";
import {
  CreateOrganization,
  OrganizationProfile,
  OrganizationSwitcher,
  SignIn,
  UserButton,
  UserProfile,
  useAuth,
  useOrganization,
  useUser,
} from "@clerk/react";
import Footer from "./components/Footer";
import { LppClerkProvider } from "./lib/clerk";
import "./index.css";

const GIT_COMMANDS = [
  {
    id: "registry",
    label: "1. Point Keel at your writable git registry (SSH for push)",
    command: "export KEEL_REGISTRY=git@github.com:samarnever-droid/llppregistry.git",
  },
  {
    id: "init",
    label: "2. Create or inspect the package manifest",
    command: "keel init  # creates Keel.toml + src/main.lpp",
  },
  {
    id: "verify",
    label: "3. Check, test, and verify locally before publishing",
    command: "keel check && keel test && keel verify",
  },
  {
    id: "publish",
    label: "4. Pack, checksum, commit, and push the release",
    command: "keel publish",
  },
  {
    id: "consumer",
    label: "5. Consumers install directly (no env var needed)",
    command: "keel add <package> && keel install && keel build",
  },
];

const STATUS = [
  { label: "Domain", value: "lplusplus.bond", tone: "text-acid" },
  { label: "Auth (Clerk Pro)", value: "clerk.lplusplus.bond", tone: "text-violet-300" },
  { label: "Registry Edge API", value: "registry.lplusplus.bond", tone: "text-emerald-300" },
  { label: "Durable Authority", value: "llppregistry (git + SHA-256)", tone: "text-sky-300" },
];

const FLOW = [
  {
    icon: Package,
    title: "Manifest",
    text: "Keel.toml declares package name, version, edition, license, features, targets, and dependencies.",
  },
  {
    icon: UploadCloud,
    title: "SHA-256 Artifact",
    text: "Keel packs the source deterministically by SHA-256 so every download is tamper-evident.",
  },
  {
    icon: GitBranch,
    title: "Dual Publishing",
    text: "Use the Clerk Publisher Studio for guided manifest & PR publishing, or push directly with `keel publish`.",
  },
  {
    icon: RefreshCw,
    title: "Cloudflare Edge Mirror",
    text: "Cloudflare Pages and registry.lplusplus.bond serve fast global search, stats, and index lookups.",
  },
];

function CopyCommand({
  id,
  label,
  command,
  copied,
  onCopy,
}: {
  id: string;
  label: string;
  command: string;
  copied: string | null;
  onCopy: (text: string, id: string) => void;
}) {
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
          {copied === id ? (
            <Check className="h-3 w-3 text-emerald-400" />
          ) : (
            <Copy className="h-3 w-3" />
          )}
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
  const { isLoaded, isSignedIn, userId, orgId, orgRole, getToken } = useAuth();
  const { user } = useUser();
  const { organization } = useOrganization();

  const [copied, setCopied] = useState<string | null>(null);
  const [studioTab, setStudioTab] = useState<"studio" | "team" | "git" | "token" | "security">(
    "studio"
  );

  // Interactive Manifest & Release Builder state
  const [pkgName, setPkgName] = useState("lpp-my-pkg");
  const [pkgVersion, setPkgVersion] = useState("0.1.0");
  const [pkgLicense, setPkgLicense] = useState("MIT");
  const [pkgDescription, setPkgDescription] = useState(
    "High-performance native library written in pure L++."
  );
  const [pkgDeps, setPkgDeps] = useState("lpp-semver = \"^1.0.0\"");

  // Live Worker health + JWT state
  const [workerHealth, setWorkerHealth] = useState<string | null>(null);
  const [checkingHealth, setCheckingHealth] = useState(false);
  const [tokenCopied, setTokenCopied] = useState(false);

  const copyText = (text: string, id: string) => {
    navigator.clipboard.writeText(text);
    setCopied(id);
    setTimeout(() => setCopied(null), 1800);
  };

  const publisherDisplay =
    organization?.name ||
    user?.fullName ||
    user?.username ||
    user?.primaryEmailAddress?.emailAddress ||
    "L++ Developer";
  const publisherHandle = user?.username ? `@${user.username}` : null;
  const publisherEmail = user?.primaryEmailAddress?.emailAddress || "";

  const generatedManifest = `[package]
name = "${pkgName.trim() || "lpp-my-pkg"}"
version = "${pkgVersion.trim() || "0.1.0"}"
edition = "2024"
license = "${pkgLicense.trim() || "MIT"}"
authors = ["${publisherDisplay}${publisherEmail ? ` <${publisherEmail}>` : ""}"]
description = "${pkgDescription.replace(/"/g, '\\"')}"
${organization?.slug ? `namespace = "${organization.slug}"\n` : ""}
[dependencies]
${pkgDeps.trim()}
`;

  const publishSnippet = `keel check && keel test && keel verify && KEEL_REGISTRY=git@github.com:samarnever-droid/llppregistry.git keel publish`;

  const githubIssueUrl = (() => {
    const title = encodeURIComponent(
      `[Publish] ${pkgName.trim() || "lpp-my-pkg"}@${pkgVersion.trim() || "0.1.0"}`
    );
    const body = encodeURIComponent(
      `### Publisher (Clerk Verified)\n- **Publisher**: ${publisherDisplay}\n- **Email**: ${publisherEmail || "N/A"}\n- **Clerk User ID**: \`${userId || "unauthenticated"}\`\n\n### Keel.toml\n\`\`\`toml\n${generatedManifest}\`\`\`\n\n### Verification\n- [ ] \`keel check && keel test && keel verify\` passed\n- [ ] SHA-256 artifact matches \`Keel.lock\`\n`
    );
    return `https://github.com/samarnever-droid/llppregistry/issues/new?title=${title}&body=${body}`;
  })();

  const handleCopySessionJwt = async () => {
    try {
      const token = await getToken();
      if (token) {
        await navigator.clipboard.writeText(token);
        setTokenCopied(true);
        setTimeout(() => setTokenCopied(false), 2000);
      }
    } catch {
      // ignore
    }
  };

  const handlePingWorker = async () => {
    setCheckingHealth(true);
    try {
      const res = await fetch("https://registry.lplusplus.bond/health");
      const data = await res.json();
      setWorkerHealth(JSON.stringify(data, null, 2));
    } catch (err) {
      setWorkerHealth(`Error contacting registry.lplusplus.bond: ${String(err)}`);
    } finally {
      setCheckingHealth(false);
    }
  };

  return (
    <div className="min-h-screen bg-[#07090d] text-white flex flex-col font-sans antialiased">
      <header className="sticky top-0 z-40 border-b border-white/10 bg-[#07090d]/90 backdrop-blur-xl">
        <div className="mx-auto flex h-16 max-w-7xl items-center justify-between px-5 md:px-8">
          <a href="/" className="flex items-center gap-2.5 text-white/80 hover:text-white">
            <ArrowLeft className="h-4 w-4" />
            <span className="font-mono text-xs">Back to Main</span>
          </a>
          <div className="flex items-center gap-3">
            {isLoaded && isSignedIn && <OrganizationSwitcher hidePersonal={false} />}
            <div className="hidden items-center gap-2 rounded-full border border-acid/25 bg-acid/10 px-3 py-1 font-mono text-xs text-acid sm:flex">
              <ShieldCheck className="h-3.5 w-3.5" />
              {isLoaded && isSignedIn ? "Clerk Pro Publisher Studio" : "Dual-Option Publishing"}
            </div>
            {isLoaded && isSignedIn && <UserButton />}
          </div>
        </div>
      </header>

      <main className="mx-auto max-w-7xl w-full flex-1 px-5 md:px-8 py-10 space-y-10">
        {/* Hero */}
        <section className="overflow-hidden rounded-3xl border border-white/10 bg-gradient-to-br from-white/[0.06] via-white/[0.025] to-acid/[0.04] p-6 md:p-10 shadow-2xl">
          <div className="grid gap-8 lg:grid-cols-[1.2fr_0.8fr] lg:items-center">
            <div className="space-y-6">
              <div className="inline-flex items-center gap-2 rounded-full border border-emerald-400/30 bg-emerald-400/10 px-3 py-1 font-mono text-xs text-emerald-300">
                <Sparkles className="h-3.5 w-3.5" />
                {isLoaded && isSignedIn
                  ? `Signed in as ${publisherDisplay}`
                  : "Clerk Pro + Durable Git Registry"}
              </div>
              <div>
                <h1 className="max-w-3xl font-mono text-3xl font-black leading-tight tracking-tight text-white sm:text-5xl">
                  Two ways to publish <span className="text-acid">L++ packages</span>
                </h1>
                <p className="mt-4 max-w-2xl text-sm leading-7 text-white/65 sm:text-base">
                  Use <span className="text-white font-semibold">Option 1 (Clerk Publisher Studio)</span> for verified developer identity, interactive manifest generation, session JWTs, MFA/passkeys, and guided registry releases — or <span className="text-acid font-semibold">Option 2 (Direct Keel Git CLI)</span> to commit &amp; push straight into <code className="text-white">llppregistry</code>.
                </p>
              </div>
              <div className="flex flex-wrap gap-3">
                <a
                  href="/packages.html"
                  className="clerk-btn-primary inline-flex items-center gap-2 rounded-xl px-4 py-2.5 font-mono text-xs font-semibold"
                >
                  Browse packages
                  <Package className="h-3.5 w-3.5" />
                </a>
                <a
                  href="https://registry.lplusplus.bond/health"
                  target="_blank"
                  rel="noreferrer"
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
                  <div className="font-mono text-[11px] uppercase tracking-wider text-white/40">
                    {item.label}
                  </div>
                  <div className={`mt-1 font-mono text-sm font-bold ${item.tone}`}>
                    {item.value}
                  </div>
                </div>
              ))}
            </div>
          </div>
        </section>

        {/* Authenticated Clerk Pro Studio OR Sign-In + Direct Git Side-by-Side */}
        {!isLoaded ? (
          <div className="rounded-3xl border border-white/10 bg-white/[0.03] p-12 text-center font-mono text-sm text-white/55">
            Initializing Clerk Pro session on clerk.lplusplus.bond...
          </div>
        ) : isSignedIn && user ? (
          <section className="space-y-6">
            {/* Studio Navigation Tabs */}
            <div className="flex flex-wrap gap-2 rounded-2xl border border-white/10 bg-white/[0.03] p-2">
              {[
                { id: "studio", label: "Publish Studio", icon: UploadCloud },
                { id: "team", label: "Team & Orgs (Clerk Pro)", icon: Users },
                { id: "git", label: "Direct Keel Git CLI", icon: GitBranch },
                { id: "token", label: "Session & Edge API", icon: Activity },
                { id: "security", label: "Passkeys, MFA & Profile", icon: Shield },
              ].map((t) => {
                const Icon = t.icon;
                return (
                  <button
                    key={t.id}
                    onClick={() => setStudioTab(t.id as typeof studioTab)}
                    className={`inline-flex items-center gap-2 rounded-xl px-4 py-2.5 font-mono text-xs font-semibold transition-all ${
                      studioTab === t.id
                        ? "clerk-btn-primary"
                        : "text-white/70 hover:bg-white/5 hover:text-white"
                    }`}
                  >
                    <Icon className="h-3.5 w-3.5" />
                    {t.label}
                  </button>
                );
              })}
            </div>

            {studioTab === "studio" && (
              <div className="grid gap-6 lg:grid-cols-2">
                <div className="rounded-3xl border border-white/10 bg-white/[0.03] p-6 space-y-4">
                  <div className="flex items-center justify-between">
                    <div>
                      <h2 className="font-mono text-lg font-bold text-white">
                        Interactive Manifest &amp; Release Builder
                      </h2>
                      <p className="font-mono text-xs text-white/45">
                        Pre-filled with your verified Clerk identity ({publisherDisplay})
                      </p>
                    </div>
                    <span className="inline-flex items-center gap-1 rounded-full border border-emerald-400/30 bg-emerald-400/10 px-2.5 py-1 font-mono text-[11px] text-emerald-300">
                      <UserCheck className="h-3 w-3" />
                      Verified
                    </span>
                  </div>

                  <div className="grid gap-3 sm:grid-cols-2">
                    <label className="space-y-1.5">
                      <span className="font-mono text-[11px] uppercase tracking-wider text-white/50">
                        Package Name
                      </span>
                      <input
                        value={pkgName}
                        onChange={(e) => setPkgName(e.target.value)}
                        className="w-full rounded-xl border border-white/15 bg-[#05070a] px-3 py-2 font-mono text-xs text-white focus:border-acid focus:outline-none"
                      />
                    </label>
                    <label className="space-y-1.5">
                      <span className="font-mono text-[11px] uppercase tracking-wider text-white/50">
                        Version (SemVer)
                      </span>
                      <input
                        value={pkgVersion}
                        onChange={(e) => setPkgVersion(e.target.value)}
                        className="w-full rounded-xl border border-white/15 bg-[#05070a] px-3 py-2 font-mono text-xs text-white focus:border-acid focus:outline-none"
                      />
                    </label>
                  </div>

                  <label className="block space-y-1.5">
                    <span className="font-mono text-[11px] uppercase tracking-wider text-white/50">
                      License
                    </span>
                    <input
                      value={pkgLicense}
                      onChange={(e) => setPkgLicense(e.target.value)}
                      className="w-full rounded-xl border border-white/15 bg-[#05070a] px-3 py-2 font-mono text-xs text-white focus:border-acid focus:outline-none"
                    />
                  </label>

                  <label className="block space-y-1.5">
                    <span className="font-mono text-[11px] uppercase tracking-wider text-white/50">
                      Description
                    </span>
                    <input
                      value={pkgDescription}
                      onChange={(e) => setPkgDescription(e.target.value)}
                      className="w-full rounded-xl border border-white/15 bg-[#05070a] px-3 py-2 font-mono text-xs text-white focus:border-acid focus:outline-none"
                    />
                  </label>

                  <label className="block space-y-1.5">
                    <span className="font-mono text-[11px] uppercase tracking-wider text-white/50">
                      Dependencies (TOML lines)
                    </span>
                    <textarea
                      rows={3}
                      value={pkgDeps}
                      onChange={(e) => setPkgDeps(e.target.value)}
                      className="w-full rounded-xl border border-white/15 bg-[#05070a] px-3 py-2 font-mono text-xs text-white focus:border-acid focus:outline-none"
                    />
                  </label>

                  <div className="flex flex-wrap gap-3 pt-2">
                    <a
                      href={githubIssueUrl}
                      target="_blank"
                      rel="noreferrer"
                      className="clerk-btn-primary inline-flex items-center gap-2 rounded-xl px-4 py-2.5 font-mono text-xs font-semibold"
                    >
                      <UploadCloud className="h-3.5 w-3.5" />
                      Submit Verified Release Request
                      <ExternalLink className="h-3.5 w-3.5" />
                    </a>
                    <button
                      onClick={() => copyText(generatedManifest, "manifest")}
                      className="inline-flex items-center gap-2 rounded-xl border border-white/15 bg-white/5 px-4 py-2.5 font-mono text-xs text-white/80 hover:border-acid/40 hover:text-acid"
                    >
                      {copied === "manifest" ? (
                        <Check className="h-3.5 w-3.5 text-emerald-400" />
                      ) : (
                        <Copy className="h-3.5 w-3.5" />
                      )}
                      {copied === "manifest" ? "Copied Keel.toml" : "Copy Keel.toml"}
                    </button>
                  </div>
                </div>

                <div className="space-y-4">
                  <div className="rounded-3xl border border-white/10 bg-white/[0.03] p-6">
                    <div className="mb-3 flex items-center justify-between">
                      <span className="font-mono text-xs font-bold uppercase tracking-wider text-white/55">
                        Generated Keel.toml
                      </span>
                      <span className="font-mono text-[11px] text-acid">live preview</span>
                    </div>
                    <pre className="overflow-x-auto rounded-2xl border border-white/10 bg-[#05070a] p-4 font-mono text-xs leading-6 text-acid">
                      {generatedManifest}
                    </pre>
                  </div>

                  <CopyCommand
                    id="studio-publish"
                    label="One-line verify & publish command"
                    command={publishSnippet}
                    copied={copied}
                    onCopy={copyText}
                  />
                </div>
              </div>
            )}

            {studioTab === "team" && (
              <div className="space-y-6">
                <div className="rounded-3xl border border-acid/20 bg-acid/[0.03] p-5 flex flex-wrap items-center justify-between gap-4">
                  <div>
                    <h2 className="font-mono text-base font-bold text-white">
                      {organization
                        ? `Active Team Workspace: ${organization.name}`
                        : "Personal Publisher Account"}
                    </h2>
                    <p className="mt-1 font-mono text-xs text-white/60">
                      {organization
                        ? `Slug: ${organization.slug || "none"} · Role: ${orgRole || "member"} · Org ID: ${orgId}`
                        : "Create or select a Clerk Pro Organization to publish shared L++ packages with team RBAC roles and member invites."}
                    </p>
                  </div>
                  <OrganizationSwitcher hidePersonal={false} />
                </div>
                <div className="flex justify-center rounded-3xl border border-white/10 bg-white/[0.02] p-4 sm:p-6">
                  {organization ? (
                    <OrganizationProfile routing="hash" />
                  ) : (
                    <CreateOrganization routing="hash" />
                  )}
                </div>
              </div>
            )}

            {studioTab === "git" && (
              <div className="space-y-3">
                {GIT_COMMANDS.map((item) => (
                  <CopyCommand key={item.id} {...item} copied={copied} onCopy={copyText} />
                ))}
              </div>
            )}

            {studioTab === "token" && (
              <div className="grid gap-6 lg:grid-cols-2">
                <div className="rounded-3xl border border-white/10 bg-white/[0.03] p-6 space-y-4">
                  <div className="flex items-center gap-3">
                    <div className="flex h-10 w-10 items-center justify-center rounded-xl border border-acid/25 bg-acid/10 text-acid">
                      <Key className="h-5 w-5" />
                    </div>
                    <div>
                      <h2 className="font-mono text-lg font-bold text-white">
                        Clerk Session &amp; API JWT
                      </h2>
                      <p className="font-mono text-xs text-white/45">
                        Signed by clerk.lplusplus.bond
                      </p>
                    </div>
                  </div>
                  <div className="space-y-2 rounded-2xl border border-white/10 bg-black/30 p-4 font-mono text-xs">
                    <div className="flex justify-between gap-2">
                      <span className="text-white/45">User ID</span>
                      <span className="text-white">{userId}</span>
                    </div>
                    {publisherHandle && (
                      <div className="flex justify-between gap-2">
                        <span className="text-white/45">Publisher Handle</span>
                        <span className="text-acid">{publisherHandle}</span>
                      </div>
                    )}
                    <div className="flex justify-between gap-2">
                      <span className="text-white/45">Primary Email</span>
                      <span className="text-acid">{publisherEmail || "None"}</span>
                    </div>
                    <div className="flex justify-between gap-2">
                      <span className="text-white/45">Active Org / Role</span>
                      <span className="text-violet-300">
                        {organization ? `${organization.name} (${orgRole || "member"})` : "Personal"}
                      </span>
                    </div>
                    <div className="flex justify-between gap-2">
                      <span className="text-white/45">Clerk Pro Features</span>
                      <span className="text-emerald-300">
                        White-label · Orgs · Passkeys · TOTP MFA · Multi-Session
                      </span>
                    </div>
                    <div className="flex justify-between gap-2">
                      <span className="text-white/45">Frontend API</span>
                      <span className="text-emerald-300">https://clerk.lplusplus.bond</span>
                    </div>
                  </div>
                  <button
                    onClick={handleCopySessionJwt}
                    className="clerk-btn-primary inline-flex items-center gap-2 rounded-xl px-4 py-2.5 font-mono text-xs font-semibold"
                  >
                    {tokenCopied ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />}
                    {tokenCopied ? "Session JWT Copied!" : "Copy Bearer Session JWT"}
                  </button>
                </div>

                <div className="rounded-3xl border border-white/10 bg-white/[0.03] p-6 space-y-4">
                  <div className="flex items-center justify-between gap-3">
                    <div className="flex items-center gap-3">
                      <div className="flex h-10 w-10 items-center justify-center rounded-xl border border-emerald-400/25 bg-emerald-400/10 text-emerald-300">
                        <Activity className="h-5 w-5" />
                      </div>
                      <div>
                        <h2 className="font-mono text-lg font-bold text-white">
                          Cloudflare Edge Registry Check
                        </h2>
                        <p className="font-mono text-xs text-white/45">
                          Live ping to registry.lplusplus.bond/health
                        </p>
                      </div>
                    </div>
                    <button
                      onClick={handlePingWorker}
                      disabled={checkingHealth}
                      className="inline-flex items-center gap-1.5 rounded-xl border border-white/15 bg-white/5 px-3 py-2 font-mono text-xs text-white/80 hover:border-acid/40 hover:text-acid disabled:opacity-50"
                    >
                      <RefreshCw className={`h-3.5 w-3.5 ${checkingHealth ? "animate-spin" : ""}`} />
                      {checkingHealth ? "Checking..." : "Ping Live Worker"}
                    </button>
                  </div>
                  <pre className="min-h-[160px] overflow-x-auto rounded-2xl border border-white/10 bg-[#05070a] p-4 font-mono text-xs leading-6 text-emerald-300">
                    {workerHealth || 'Click "Ping Live Worker" to inspect registry.lplusplus.bond/health'}
                  </pre>
                </div>
              </div>
            )}

            {studioTab === "security" && (
              <div className="flex justify-center rounded-3xl border border-white/10 bg-white/[0.02] p-4 sm:p-6">
                <UserProfile routing="hash" />
              </div>
            )}
          </section>
        ) : (
          <section className="grid gap-8 lg:grid-cols-[1fr_1fr] lg:items-start">
            <div className="rounded-3xl border border-acid/25 bg-white/[0.03] p-6 space-y-5">
              <div className="flex items-center gap-3">
                <div className="flex h-10 w-10 items-center justify-center rounded-xl border border-acid/30 bg-acid/10 text-acid">
                  <Lock className="h-5 w-5" />
                </div>
                <div>
                  <h2 className="font-mono text-lg font-bold text-white">
                    Option 1 — Clerk Pro Publisher Studio
                  </h2>
                  <p className="font-mono text-xs text-white/50">
                    Sign in via clerk.lplusplus.bond (Passkeys, MFA, OAuth)
                  </p>
                </div>
              </div>
              <div className="flex justify-center">
                <SignIn routing="hash" />
              </div>
            </div>

            <div className="space-y-4">
              <div className="rounded-3xl border border-white/10 bg-white/[0.03] p-6">
                <div className="mb-4 flex items-center gap-3">
                  <div className="flex h-10 w-10 items-center justify-center rounded-xl border border-emerald-400/25 bg-emerald-400/10 text-emerald-300">
                    <GitBranch className="h-5 w-5" />
                  </div>
                  <div>
                    <h2 className="font-mono text-lg font-bold text-white">
                      Option 2 — Direct Keel Git CLI
                    </h2>
                    <p className="font-mono text-xs text-white/50">
                      Zero browser login required — push straight to llppregistry
                    </p>
                  </div>
                </div>
                <div className="space-y-3">
                  {GIT_COMMANDS.map((item) => (
                    <CopyCommand key={item.id} {...item} copied={copied} onCopy={copyText} />
                  ))}
                </div>
              </div>
            </div>
          </section>
        )}

        {/* Architecture Flow */}
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

        {/* Runtime surfaces */}
        <section className="rounded-3xl border border-white/10 bg-white/[0.03] p-6 md:p-8">
          <div className="mb-6 flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-xl border border-acid/25 bg-acid/10 text-acid">
              <Layers className="h-5 w-5" />
            </div>
            <div>
              <h2 className="font-mono text-lg font-bold text-white">Live Infrastructure</h2>
              <p className="font-mono text-xs text-white/45">
                Cloudflare Pages + Worker + Clerk Pro custom domain
              </p>
            </div>
          </div>
          <div className="grid gap-3 md:grid-cols-2">
            <div className="rounded-2xl border border-white/10 bg-black/25 p-4">
              <div className="font-mono text-xs text-white/40">Cloudflare &amp; Clerk endpoints</div>
              <pre className="mt-3 overflow-x-auto rounded-xl bg-[#05070a] p-3 font-mono text-xs leading-6 text-acid">{`DOMAIN=lplusplus.bond
CLERK_FRONTEND_API=https://clerk.lplusplus.bond
REGISTRY_URL=https://registry.lplusplus.bond
GITHUB_REPO=samarnever-droid/llppregistry`}</pre>
            </div>
            <div className="rounded-2xl border border-white/10 bg-black/25 p-4">
              <div className="font-mono text-xs text-white/40">Security model</div>
              <p className="mt-3 text-sm leading-7 text-white/60">
                Clerk Pro secures publisher identities, passkeys, MFA, and sessions on <code className="text-white">clerk.lplusplus.bond</code>. Package artifacts remain immutable and SHA-256 verified in <code className="text-acid">Keel.lock</code>.
              </p>
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
    <LppClerkProvider>
      <AccountApp />
    </LppClerkProvider>
  </React.StrictMode>
);
