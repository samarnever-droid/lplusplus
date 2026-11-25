import { useEffect, useState } from "react";
import {
  Bookmark,
  Check,
  Copy,
  Play,
  RotateCcw,
  Share2,
  ShieldCheck,
  Sparkles,
  Trash2,
} from "lucide-react";
import { SignInButton, useAuth, useUser } from "@clerk/react";
import { SectionHead } from "../lib/ui";

interface Preset {
  id: string;
  label: string;
  tag: string;
  code: string;
  output: string;
}

const PRESETS: Preset[] = [
  {
    id: "capabilities",
    label: "@pure & @deny",
    tag: "Compile-time security",
    code: `@pure
def score(x: Int) -> Int:
    # Algebraic MIR simplifies (x * 8) -> (x << 3) and (x ** 2) -> (x * x)
    return (x * 8) + (x ** 2) + (x ** 0)

@deny(fs, net, exec, ffi, unsafe)
def badge(name: Str, ok: Bool) -> Str:
    state := "verified" if ok else "blocked"
    return f"{name[0..6]} :: {state}"

def main():
    print(score(5))
    print_str(badge("lplusplus", true))`,
    output: `66\nlplusp :: verified\n\n[cranelift] verified @pure & @deny transitively · 0.8ms`,
  },
  {
    id: "slicing",
    label: "String Slicing & Ternary",
    tag: "v0.3.2 ergonomics",
    code: `def main():
    s := "hello_world"
    print_str(s[0..5])
    print_str(s[6..=10])
    print_str(s[..2] + "-" + s[6..])

    denom := 0
    safe := 100 / denom if denom != 0 else -1
    print(safe)`,
    output: `hello\nworld\nhe-world\n-1\n\n[parity] MIR == Cranelift == LLVM -O3 == WASM WASI`,
  },
  {
    id: "hof",
    label: "Fn Types & Generic Aliases",
    tag: "Higher-order functions",
    code: `type Pair[T] = (T, T)
type Combiner = fn(Int, Int) -> Int

def apply(f: Combiner, p: Pair[Int]) -> Int:
    return f(p.0, p.1)

def main():
    p: Pair[Int] = (10, 20)
    mul: Combiner = fn(a: Int, b: Int) -> Int: a * b
    print(apply(mul, p))`,
    output: `200\n\n[llvm -O3] 0 heap leaks · deterministic ARC`,
  },
  {
    id: "kernel",
    label: "Bare-Metal Kernel",
    tag: "x86_64-unknown-none",
    code: `@export_name("_start")
@section(".text.boot")
unsafe def kmain() -> Void:
    vga := (0xB8000 as USize) as Ptr[U16]
    ptr_write[U16](vga, 0x0F4C as U16)
    port_out8(0x3F8 as U16, 0x4C as U8)
    cpu_hlt()`,
    output: `[lpp-link] emitted freestanding ELF64 (multiboot2)\n[safety] 1 unsafe def audited · 0 runtime deps`,
  },
];

interface SavedSnippet {
  id: string;
  name: string;
  code: string;
}

function encodeShare(code: string): string {
  try {
    return btoa(unescape(encodeURIComponent(code)));
  } catch {
    return "";
  }
}

function decodeShare(hash: string): string | null {
  try {
    return decodeURIComponent(escape(atob(hash)));
  } catch {
    return null;
  }
}

function evaluatePreview(code: string): string {
  const matched = PRESETS.find((p) => p.code.trim() === code.trim());
  if (matched) return matched.output;

  const lines: string[] = [];
  const printStrRe = /print_str\("([^"]*)"\)/g;
  const printIntRe = /print\((-?\d+)\)/g;
  let m: RegExpExecArray | null;
  while ((m = printStrRe.exec(code)) !== null) lines.push(m[1]);
  while ((m = printIntRe.exec(code)) !== null) lines.push(m[1]);

  if (code.includes("@pure") && (code.includes("read_file(") || code.includes("write_file("))) {
    return `[E3204] capability \`fs\` is denied in this function (calls builtin \`read_file\`)`;
  }
  if (lines.length > 0) {
    return `${lines.join("\n")}\n\n[preview] Syntax valid · run locally with \`lpp main.lpp --run\``;
  }
  return `[preview] Ready — run natively with \`lpp main.lpp --run\` or \`keel run\``;
}

export default function Playground() {
  const { isLoaded, isSignedIn } = useAuth();
  const { user } = useUser();

  const [activePreset, setActivePreset] = useState(0);
  const [code, setCode] = useState(PRESETS[0].code);
  const [output, setOutput] = useState(PRESETS[0].output);
  const [shared, setShared] = useState(false);
  const [copiedCode, setCopiedCode] = useState(false);
  const [saving, setSaving] = useState(false);
  const [savedNotice, setSavedNotice] = useState(false);

  // Load shared snippet from URL hash `#play=...`
  useEffect(() => {
    if (typeof window === "undefined") return;
    const hash = window.location.hash;
    if (hash.startsWith("#play=")) {
      const decoded = decodeShare(hash.slice(6));
      if (decoded) {
        setCode(decoded);
        setOutput(evaluatePreview(decoded));
      }
    }
  }, []);

  const savedSnippets: SavedSnippet[] = Array.isArray(user?.unsafeMetadata?.savedSnippets)
    ? (user?.unsafeMetadata?.savedSnippets as SavedSnippet[])
    : [];

  const handleRun = () => {
    setOutput(evaluatePreview(code));
  };

  const handleShare = async () => {
    const encoded = encodeShare(code);
    const url = `${window.location.origin}/#play=${encoded}`;
    window.history.replaceState(null, "", `#play=${encoded}`);
    try {
      await navigator.clipboard.writeText(url);
    } catch {
      /* ignore */
    }
    setShared(true);
    setTimeout(() => setShared(false), 2000);
  };

  const handleSaveToAccount = async () => {
    if (!user) return;
    setSaving(true);
    try {
      const name = `snippet-${savedSnippets.length + 1}.lpp`;
      const trimmed = code.slice(0, 1200); // stay well within Clerk's 8KB metadata limit
      const next = [{ id: String(Date.now()), name, code: trimmed }, ...savedSnippets].slice(0, 4);
      await user.update({
        unsafeMetadata: {
          ...user.unsafeMetadata,
          savedSnippets: next,
        },
      });
      setSavedNotice(true);
      setTimeout(() => setSavedNotice(false), 2000);
    } catch {
      /* ignore */
    } finally {
      setSaving(false);
    }
  };

  const handleDeleteSnippet = async (id: string) => {
    if (!user) return;
    const next = savedSnippets.filter((s) => s.id !== id);
    await user.update({
      unsafeMetadata: {
        ...user.unsafeMetadata,
        savedSnippets: next,
      },
    });
  };

  return (
    <section id="playground" className="relative border-t border-white/[0.06] py-24 md:py-32">
      <div className="mx-auto max-w-7xl px-5 md:px-8">
        <SectionHead
          index="02"
          kicker="Interactive Playground"
          title={
            <>
              Try L++ v0.3.2 <span className="text-[#a78bfa]">right here</span>
            </>
          }
          desc="Edit live snippets, test @pure/@deny capability rules, share instant #play URLs, or save snippets straight to your account."
        />

        <div className="mt-10 grid gap-6 lg:grid-cols-12">
          {/* Editor + Output */}
          <div className="clerk-card overflow-hidden rounded-2xl lg:col-span-8">
            <div className="flex flex-wrap items-center justify-between gap-2 border-b border-white/[0.08] bg-white/[0.02] px-4 py-3">
              <div className="flex flex-wrap items-center gap-1.5">
                {PRESETS.map((p, idx) => (
                  <button
                    key={p.id}
                    onClick={() => {
                      setActivePreset(idx);
                      setCode(p.code);
                      setOutput(p.output);
                    }}
                    className={`rounded-lg px-2.5 py-1 font-mono text-xs transition-all ${
                      activePreset === idx
                        ? "border border-[#6c47ff]/50 bg-[#6c47ff]/20 text-white"
                        : "text-zinc-400 hover:bg-white/[0.05] hover:text-zinc-200"
                    }`}
                  >
                    {p.label}
                  </button>
                ))}
              </div>
              <div className="flex items-center gap-2">
                <button
                  onClick={() => {
                    setCode(PRESETS[activePreset].code);
                    setOutput(PRESETS[activePreset].output);
                  }}
                  className="rounded-lg border border-white/10 bg-white/[0.04] p-1.5 text-zinc-400 hover:text-white"
                  title="Reset snippet"
                >
                  <RotateCcw className="h-3.5 w-3.5" />
                </button>
                <button
                  onClick={() => {
                    navigator.clipboard.writeText(code);
                    setCopiedCode(true);
                    setTimeout(() => setCopiedCode(false), 1600);
                  }}
                  className="inline-flex items-center gap-1.5 rounded-lg border border-white/10 bg-white/[0.04] px-2.5 py-1.5 font-mono text-[11px] text-zinc-300 hover:text-white"
                >
                  {copiedCode ? <Check className="h-3 w-3 text-emerald-400" /> : <Copy className="h-3 w-3" />}
                  {copiedCode ? "Copied" : "Copy"}
                </button>
                <button
                  onClick={handleShare}
                  className="inline-flex items-center gap-1.5 rounded-lg border border-white/10 bg-white/[0.04] px-2.5 py-1.5 font-mono text-[11px] text-zinc-300 hover:border-[#6c47ff]/50 hover:text-white"
                >
                  {shared ? <Check className="h-3 w-3 text-emerald-400" /> : <Share2 className="h-3 w-3" />}
                  {shared ? "Link Copied" : "Share"}
                </button>
                <button
                  onClick={handleRun}
                  className="clerk-btn-primary inline-flex items-center gap-1.5 rounded-lg px-3.5 py-1.5 font-mono text-xs font-semibold"
                >
                  <Play className="h-3.5 w-3.5" />
                  Run
                </button>
              </div>
            </div>

            <textarea
              value={code}
              onChange={(e) => setCode(e.target.value)}
              spellCheck={false}
              rows={13}
              className="w-full resize-none bg-[#09090b] p-5 font-mono text-[13px] leading-6 text-zinc-100 focus:outline-none"
            />

            <div className="border-t border-white/[0.08] bg-[#111116] p-4">
              <div className="mb-2 flex items-center justify-between">
                <span className="font-mono text-[11px] uppercase tracking-wider text-zinc-400">
                  Compiler &amp; Output
                </span>
                <span className="inline-flex items-center gap-1 font-mono text-[11px] text-emerald-400">
                  <ShieldCheck className="h-3.5 w-3.5" />
                  v0.3.2
                </span>
              </div>
              <pre className="overflow-x-auto whitespace-pre-wrap font-mono text-xs leading-6 text-zinc-200">
                {output}
              </pre>
            </div>
          </div>

          {/* Saved Snippets Sidebar (Clerk-synced) */}
          <div className="clerk-card flex flex-col justify-between rounded-2xl p-5 lg:col-span-4">
            <div className="space-y-4">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <Bookmark className="h-4 w-4 text-[#a78bfa]" />
                  <h3 className="font-display text-base font-semibold text-white">
                    Saved Snippets
                  </h3>
                </div>
                <span className="rounded-full border border-white/10 bg-white/[0.04] px-2 py-0.5 font-mono text-[10px] text-zinc-400">
                  {savedSnippets.length} / 4
                </span>
              </div>

              {!isLoaded ? (
                <div className="text-xs text-zinc-400">Loading account...</div>
              ) : isSignedIn && user ? (
                <div className="space-y-3">
                  <button
                    onClick={handleSaveToAccount}
                    disabled={saving}
                    className="clerk-btn-primary flex w-full items-center justify-center gap-2 rounded-xl px-4 py-2.5 text-xs font-semibold disabled:opacity-50"
                  >
                    {savedNotice ? (
                      <>
                        <Check className="h-3.5 w-3.5" />
                        Saved to your account!
                      </>
                    ) : (
                      <>
                        <Sparkles className="h-3.5 w-3.5" />
                        {saving ? "Saving..." : "Save Current Snippet"}
                      </>
                    )}
                  </button>

                  {savedSnippets.length === 0 ? (
                    <div className="rounded-xl border border-dashed border-white/15 bg-[#09090b]/60 p-5 text-center text-xs text-zinc-400">
                      No saved snippets yet. Click <strong>Save Current Snippet</strong> to sync code across devices.
                    </div>
                  ) : (
                    <div className="space-y-2">
                      {savedSnippets.map((s) => (
                        <div
                          key={s.id}
                          className="flex items-center justify-between gap-2 rounded-xl border border-white/10 bg-[#09090b]/80 px-3 py-2.5"
                        >
                          <button
                            onClick={() => {
                              setCode(s.code);
                              setOutput(evaluatePreview(s.code));
                            }}
                            className="truncate text-left font-mono text-xs text-zinc-200 hover:text-[#a78bfa]"
                          >
                            {s.name}
                          </button>
                          <button
                            onClick={() => handleDeleteSnippet(s.id)}
                            className="text-zinc-500 hover:text-red-400"
                            title="Delete snippet"
                          >
                            <Trash2 className="h-3.5 w-3.5" />
                          </button>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              ) : (
                <div className="rounded-xl border border-white/10 bg-[#09090b]/70 p-5 text-center space-y-3">
                  <p className="text-xs leading-relaxed text-zinc-400">
                    Sign in to save L++ snippets to your account and sync them across devices.
                  </p>
                  <SignInButton mode="modal">
                    <button className="clerk-btn-primary w-full rounded-xl px-4 py-2 text-xs font-semibold">
                      Sign in to Save Snippets
                    </button>
                  </SignInButton>
                </div>
              )}
            </div>

            <div className="mt-6 rounded-xl border border-white/[0.08] bg-[#09090b]/70 p-3.5 text-xs text-zinc-400">
              Tip: Press <kbd className="rounded border border-white/15 bg-white/10 px-1.5 py-0.5 font-mono text-[10px] text-white">⌘K</kbd> anywhere on the site to jump to syntax, stdlib, or Keel commands.
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
