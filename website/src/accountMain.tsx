import React, { useState } from "react";
import ReactDOM from "react-dom/client";
import {
  ArrowLeft,
  Check,
  Copy,
  ExternalLink,
  GitBranch,
  Lock,
  Package,
  Shield,
  ShieldCheck,
  Sparkles,
  Terminal,
  UploadCloud,
  UserCheck,
  Users,
} from "lucide-react";
import {
  AuthenticateWithRedirectCallback,
  CreateOrganization,
  OrganizationProfile,
  OrganizationSwitcher,
  SignIn,
  SignUp,
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
    id: "init",
    label: "1. Initialize a package",
    command: "keel init",
  },
  {
    id: "verify",
    label: "2. Check, test, and verify locally",
    command: "keel check && keel test && keel verify",
  },
  {
    id: "publish",
    label: "3. Publish release to the registry",
    command: "keel publish",
  },
  {
    id: "consumer",
    label: "4. Install in any L++ project",
    command: "keel add <package> && keel install && keel build",
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
    <div className="rounded-2xl border border-white/10 bg-[#09090b]/80 p-4">
      <div className="mb-2 flex items-center justify-between gap-3">
        <div className="flex items-center gap-2 font-mono text-xs font-medium text-zinc-400">
          <Terminal className="h-3.5 w-3.5 text-[#a78bfa]" />
          {label}
        </div>
        <button
          onClick={() => onCopy(command, id)}
          className="inline-flex items-center gap-1.5 rounded-lg border border-white/10 bg-white/[0.04] px-2.5 py-1 font-mono text-[11px] text-zinc-300 hover:border-[#6c47ff]/50 hover:text-white"
        >
          {copied === id ? (
            <Check className="h-3 w-3 text-emerald-400" />
          ) : (
            <Copy className="h-3 w-3" />
          )}
          {copied === id ? "Copied" : "Copy"}
        </button>
      </div>
      <pre className="overflow-x-auto rounded-xl border border-white/[0.08] bg-[#050507] p-3 font-mono text-xs text-zinc-100">
        {command}
      </pre>
    </div>
  );
}

function AccountApp() {
  const { isLoaded, isSignedIn, orgRole } = useAuth();
  const { user } = useUser();
  const { organization } = useOrganization();

  const [copied, setCopied] = useState<string | null>(null);
  const [authMode, setAuthMode] = useState<"sign-in" | "sign-up">(() =>
    typeof window !== "undefined" && window.location.pathname.includes("sign-up")
      ? "sign-up"
      : "sign-in"
  );
  const [studioTab, setStudioTab] = useState<"studio" | "team" | "cli" | "security">("studio");

  // Interactive Manifest & Release Builder state
  const [pkgName, setPkgName] = useState("lpp-my-pkg");
  const [pkgVersion, setPkgVersion] = useState("0.1.0");
  const [pkgLicense, setPkgLicense] = useState("MIT");
  const [pkgDescription, setPkgDescription] = useState(
    "High-performance native library written in pure L++."
  );
  const [pkgDeps, setPkgDeps] = useState('lpp-semver = "^1.0.0"');

  if (typeof window !== "undefined" && window.location.pathname.includes("sso-callback")) {
    return (
      <div className="min-h-screen bg-[#09090b] text-white flex items-center justify-center p-6">
        <AuthenticateWithRedirectCallback
          signInFallbackRedirectUrl="/account.html"
          signUpFallbackRedirectUrl="/account.html"
        />
      </div>
    );
  }

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

  const publishSnippet = "keel check && keel test && keel verify && keel publish";

  const githubIssueUrl = (() => {
    const title = encodeURIComponent(
      `[Publish] ${pkgName.trim() || "lpp-my-pkg"}@${pkgVersion.trim() || "0.1.0"}`
    );
    const body = encodeURIComponent(
      `### Publisher\n- **Author**: ${publisherDisplay}\n- **Email**: ${publisherEmail || "N/A"}\n\n### Keel.toml\n\`\`\`toml\n${generatedManifest}\`\`\`\n`
    );
    return `https://github.com/samarnever-droid/llppregistry/issues/new?title=${title}&body=${body}`;
  })();

  return (
    <div className="min-h-screen bg-[#09090b] text-white flex flex-col font-sans antialiased">
      <header className="sticky top-0 z-40 border-b border-white/[0.08] bg-[#09090b]/85 backdrop-blur-xl">
        <div className="mx-auto flex h-16 max-w-7xl items-center justify-between px-5 md:px-8">
          <a href="/" className="flex items-center gap-2 text-zinc-300 hover:text-white">
            <ArrowLeft className="h-4 w-4" />
            <span className="text-sm font-medium">Back to L++</span>
          </a>
          <div className="flex items-center gap-3">
            {isLoaded && isSignedIn && <OrganizationSwitcher hidePersonal={false} />}
            <a
              href="/packages.html"
              className="hidden items-center gap-1.5 rounded-lg border border-white/10 bg-white/[0.04] px-3 py-1.5 text-xs font-medium text-zinc-300 hover:text-white sm:inline-flex"
            >
              <Package className="h-3.5 w-3.5 text-[#a78bfa]" />
              Packages
            </a>
            {isLoaded && isSignedIn && <UserButton />}
          </div>
        </div>
      </header>

      <main className="mx-auto max-w-7xl w-full flex-1 px-5 md:px-8 py-10 space-y-8">
        {/* Clean Header */}
        <section className="clerk-card rounded-3xl p-6 md:p-9">
          <div className="flex flex-wrap items-center justify-between gap-6">
            <div className="space-y-3 max-w-2xl">
              <div className="inline-flex items-center gap-2 rounded-full border border-[#6c47ff]/35 bg-[#6c47ff]/10 px-3 py-1 text-xs font-medium text-[#a78bfa]">
                <Sparkles className="h-3.5 w-3.5" />
                {isLoaded && isSignedIn
                  ? `Workspace · ${publisherDisplay}`
                  : "L++ Developer Portal"}
              </div>
              <h1 className="font-display text-3xl font-semibold tracking-tight text-white sm:text-4xl">
                {isLoaded && isSignedIn
                  ? "Publish & manage your L++ packages"
                  : "Sign in to L++ Publisher Studio"}
              </h1>
              <p className="text-sm leading-relaxed text-zinc-400 sm:text-base">
                Create manifests, collaborate with organizations, verify SHA-256 releases, or publish straight from the Keel CLI.
              </p>
            </div>
            <div className="flex flex-wrap gap-3">
              <a
                href="/packages.html"
                className="clerk-btn-primary inline-flex items-center gap-2 rounded-xl px-4 py-2.5 text-xs font-semibold"
              >
                <Package className="h-3.5 w-3.5" />
                Explore Registry
              </a>
              <a
                href="/academy.html"
                className="inline-flex items-center gap-2 rounded-xl border border-white/15 bg-white/[0.04] px-4 py-2.5 text-xs font-medium text-zinc-200 hover:bg-white/[0.08]"
              >
                L++ Academy
              </a>
            </div>
          </div>
        </section>

        {!isLoaded ? (
          <div className="clerk-card rounded-3xl p-12 text-center font-mono text-sm text-zinc-400">
            Loading your workspace...
          </div>
        ) : isSignedIn && user ? (
          <section className="space-y-6">
            {/* Navigation Tabs */}
            <div className="flex flex-wrap gap-2 rounded-2xl border border-white/[0.08] bg-[#111116] p-1.5">
              {[
                { id: "studio", label: "Publish Studio", icon: UploadCloud },
                { id: "team", label: "Teams & Orgs", icon: Users },
                { id: "cli", label: "Keel CLI", icon: GitBranch },
                { id: "security", label: "Account & Security", icon: Shield },
              ].map((t) => {
                const Icon = t.icon;
                return (
                  <button
                    key={t.id}
                    onClick={() => setStudioTab(t.id as typeof studioTab)}
                    className={`inline-flex items-center gap-2 rounded-xl px-4 py-2 text-xs font-medium transition-all ${
                      studioTab === t.id
                        ? "clerk-btn-primary"
                        : "text-zinc-400 hover:bg-white/[0.05] hover:text-white"
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
                <div className="clerk-card rounded-3xl p-6 space-y-4">
                  <div className="flex items-center justify-between">
                    <div>
                      <h2 className="font-display text-lg font-semibold text-white">
                        Manifest &amp; Release Builder
                      </h2>
                      <p className="text-xs text-zinc-400">
                        Authoring as {publisherDisplay}
                      </p>
                    </div>
                    <span className="inline-flex items-center gap-1 rounded-full border border-emerald-400/30 bg-emerald-400/10 px-2.5 py-1 font-mono text-[11px] text-emerald-300">
                      <UserCheck className="h-3 w-3" />
                      Verified
                    </span>
                  </div>

                  <div className="grid gap-3 sm:grid-cols-2">
                    <label className="space-y-1.5">
                      <span className="text-xs font-medium text-zinc-400">Package Name</span>
                      <input
                        value={pkgName}
                        onChange={(e) => setPkgName(e.target.value)}
                        className="w-full rounded-xl border border-white/15 bg-[#09090b] px-3 py-2 font-mono text-xs text-white focus:border-[#6c47ff] focus:outline-none"
                      />
                    </label>
                    <label className="space-y-1.5">
                      <span className="text-xs font-medium text-zinc-400">Version</span>
                      <input
                        value={pkgVersion}
                        onChange={(e) => setPkgVersion(e.target.value)}
                        className="w-full rounded-xl border border-white/15 bg-[#09090b] px-3 py-2 font-mono text-xs text-white focus:border-[#6c47ff] focus:outline-none"
                      />
                    </label>
                  </div>

                  <label className="block space-y-1.5">
                    <span className="text-xs font-medium text-zinc-400">License</span>
                    <input
                      value={pkgLicense}
                      onChange={(e) => setPkgLicense(e.target.value)}
                      className="w-full rounded-xl border border-white/15 bg-[#09090b] px-3 py-2 font-mono text-xs text-white focus:border-[#6c47ff] focus:outline-none"
                    />
                  </label>

                  <label className="block space-y-1.5">
                    <span className="text-xs font-medium text-zinc-400">Description</span>
                    <input
                      value={pkgDescription}
                      onChange={(e) => setPkgDescription(e.target.value)}
                      className="w-full rounded-xl border border-white/15 bg-[#09090b] px-3 py-2 font-mono text-xs text-white focus:border-[#6c47ff] focus:outline-none"
                    />
                  </label>

                  <label className="block space-y-1.5">
                    <span className="text-xs font-medium text-zinc-400">Dependencies</span>
                    <textarea
                      rows={3}
                      value={pkgDeps}
                      onChange={(e) => setPkgDeps(e.target.value)}
                      className="w-full rounded-xl border border-white/15 bg-[#09090b] px-3 py-2 font-mono text-xs text-white focus:border-[#6c47ff] focus:outline-none"
                    />
                  </label>

                  <div className="flex flex-wrap gap-3 pt-2">
                    <a
                      href={githubIssueUrl}
                      target="_blank"
                      rel="noreferrer"
                      className="clerk-btn-primary inline-flex items-center gap-2 rounded-xl px-4 py-2.5 text-xs font-semibold"
                    >
                      <UploadCloud className="h-3.5 w-3.5" />
                      Submit Release
                      <ExternalLink className="h-3.5 w-3.5" />
                    </a>
                    <button
                      onClick={() => copyText(generatedManifest, "manifest")}
                      className="inline-flex items-center gap-2 rounded-xl border border-white/15 bg-white/[0.04] px-4 py-2.5 text-xs font-medium text-zinc-200 hover:bg-white/[0.08]"
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
                  <div className="clerk-card rounded-3xl p-6">
                    <div className="mb-3 flex items-center justify-between">
                      <span className="font-mono text-xs font-medium text-zinc-400">
                        Keel.toml
                      </span>
                      <span className="font-mono text-[11px] text-[#a78bfa]">live preview</span>
                    </div>
                    <pre className="overflow-x-auto rounded-2xl border border-white/[0.08] bg-[#09090b] p-4 font-mono text-xs leading-6 text-zinc-200">
                      {generatedManifest}
                    </pre>
                  </div>

                  <CopyCommand
                    id="studio-publish"
                    label="Verify & publish from terminal"
                    command={publishSnippet}
                    copied={copied}
                    onCopy={copyText}
                  />
                </div>
              </div>
            )}

            {studioTab === "team" && (
              <div className="space-y-6">
                <div className="clerk-card rounded-3xl p-5 flex flex-wrap items-center justify-between gap-4">
                  <div>
                    <h2 className="font-display text-base font-semibold text-white">
                      {organization
                        ? `Team Workspace: ${organization.name}`
                        : "Personal Workspace"}
                    </h2>
                    <p className="mt-1 text-xs text-zinc-400">
                      {organization
                        ? `Namespace: ${organization.slug || "none"} · Role: ${orgRole || "member"}`
                        : "Create or switch to an organization to collaborate on shared L++ packages."}
                    </p>
                  </div>
                  <OrganizationSwitcher hidePersonal={false} />
                </div>
                <div className="flex justify-center rounded-3xl border border-white/[0.08] bg-[#111116]/60 p-4 sm:p-6">
                  {organization ? (
                    <OrganizationProfile routing="hash" />
                  ) : (
                    <CreateOrganization routing="hash" />
                  )}
                </div>
              </div>
            )}

            {studioTab === "cli" && (
              <div className="space-y-3">
                {GIT_COMMANDS.map((item) => (
                  <CopyCommand key={item.id} {...item} copied={copied} onCopy={copyText} />
                ))}
              </div>
            )}

            {studioTab === "security" && (
              <div className="flex justify-center rounded-3xl border border-white/[0.08] bg-[#111116]/60 p-4 sm:p-6">
                <UserProfile routing="hash" />
              </div>
            )}
          </section>
        ) : (
          <section className="grid gap-8 lg:grid-cols-2 lg:items-start">
            <div className="clerk-card rounded-3xl p-6 space-y-5">
              <div className="flex items-center gap-3">
                <div className="flex h-10 w-10 items-center justify-center rounded-xl border border-[#6c47ff]/40 bg-[#6c47ff]/15 text-[#a78bfa]">
                  <Lock className="h-5 w-5" />
                </div>
                <div>
                  <h2 className="font-display text-lg font-semibold text-white">
                    Developer Account
                  </h2>
                  <p className="text-xs text-zinc-400">
                    Sign in with GitHub, Email, or Passkey
                  </p>
                </div>
              </div>
              <div className="grid grid-cols-2 gap-2 rounded-xl border border-white/10 bg-[#09090b] p-1">
                <button
                  onClick={() => setAuthMode("sign-in")}
                  className={`rounded-lg px-3 py-2 text-xs font-semibold transition-all ${
                    authMode === "sign-in"
                      ? "clerk-btn-primary"
                      : "text-zinc-400 hover:text-white"
                  }`}
                >
                  Sign In
                </button>
                <button
                  onClick={() => setAuthMode("sign-up")}
                  className={`rounded-lg px-3 py-2 text-xs font-semibold transition-all ${
                    authMode === "sign-up"
                      ? "clerk-btn-primary"
                      : "text-zinc-400 hover:text-white"
                  }`}
                >
                  Sign Up
                </button>
              </div>
              <div className="flex justify-center">
                {authMode === "sign-in" ? (
                  <SignIn
                    routing="hash"
                    signUpUrl="/sign-up"
                    fallbackRedirectUrl="/account.html"
                    signUpFallbackRedirectUrl="/account.html"
                  />
                ) : (
                  <SignUp
                    routing="hash"
                    signInUrl="/sign-in"
                    fallbackRedirectUrl="/account.html"
                    signInFallbackRedirectUrl="/account.html"
                  />
                )}
              </div>
            </div>

            <div className="clerk-card rounded-3xl p-6 space-y-4">
              <div className="flex items-center gap-3">
                <div className="flex h-10 w-10 items-center justify-center rounded-xl border border-emerald-400/25 bg-emerald-400/10 text-emerald-300">
                  <ShieldCheck className="h-5 w-5" />
                </div>
                <div>
                  <h2 className="font-display text-lg font-semibold text-white">
                    Publish with Keel CLI
                  </h2>
                  <p className="text-xs text-zinc-400">
                    Deterministic SHA-256 builds straight from your terminal
                  </p>
                </div>
              </div>
              <div className="space-y-3">
                {GIT_COMMANDS.map((item) => (
                  <CopyCommand key={item.id} {...item} copied={copied} onCopy={copyText} />
                ))}
              </div>
            </div>
          </section>
        )}
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
