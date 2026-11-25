import { useEffect, useState } from "react";
import { motion } from "framer-motion";
import { Menu, X, ArrowUpRight, Package, Search, Sparkles } from "lucide-react";
import {
  OrganizationSwitcher,
  SignInButton,
  SignUpButton,
  UserButton,
  useAuth,
} from "@clerk/react";
import { EASE } from "../lib/ui";
import CommandPalette from "./CommandPalette";

const LINKS = [
  { label: "Playground", href: `${import.meta.env.BASE_URL}#playground` },
  { label: "Academy", href: `${import.meta.env.BASE_URL}academy.html` },
  { label: "Packages", href: `${import.meta.env.BASE_URL}packages.html` },
  { label: "Memory", href: "#memory" },
  { label: "Speed", href: "#performance" },
  { label: "Install", href: "#install" },
];

interface NavProps {
  onOpenRegistry?: () => void;
  onOpenAuth?: () => void;
}

export default function Nav({ onOpenRegistry, onOpenAuth }: NavProps) {
  const [scrolled, setScrolled] = useState(false);
  const [open, setOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const { isLoaded, isSignedIn } = useAuth();

  useEffect(() => {
    const fn = () => setScrolled(window.scrollY > 16);
    fn();
    window.addEventListener("scroll", fn, { passive: true });
    return () => window.removeEventListener("scroll", fn);
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen((prev) => !prev);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const openAuth = () => {
    setOpen(false);
    onOpenAuth?.();
  };

  return (
    <motion.header
      initial={{ y: -40, opacity: 0 }}
      animate={{ y: 0, opacity: 1 }}
      transition={{ duration: 0.65, ease: EASE }}
      className={`fixed inset-x-0 top-0 z-50 transition-all duration-300 ${
        scrolled
          ? "border-b border-white/[0.08] bg-[#09090b]/85 backdrop-blur-xl"
          : "border-b border-white/[0.04] bg-[#09090b]/50 backdrop-blur-md"
      }`}
    >
      <nav className="mx-auto flex h-16 max-w-7xl items-center justify-between px-5 md:px-8">
        {/* Brand */}
        <a href={`${import.meta.env.BASE_URL}`} className="group flex items-center gap-2.5">
          <div className="flex h-8 w-8 items-center justify-center rounded-lg border border-[#6c47ff]/40 bg-[#6c47ff]/15 shadow-inner shadow-[#6c47ff]/20 transition-transform duration-300 group-hover:scale-105">
            <img
              src={`${import.meta.env.BASE_URL}lpp-icon-16.svg`}
              alt="L++"
              className="h-5 w-5"
            />
          </div>
          <span className="font-display text-[15px] font-semibold tracking-tight text-white">
            L++
          </span>
          <span className="hidden rounded-full border border-white/10 bg-white/[0.04] px-2 py-0.5 font-mono text-[10px] text-zinc-400 sm:inline-block">
            v0.3.2
          </span>
        </a>

        {/* Center Links (Clerk pill style) */}
        <div className="hidden items-center gap-1 rounded-full border border-white/[0.07] bg-white/[0.025] px-2 py-1 lg:flex">
          {LINKS.map((l) => (
            <a
              key={l.href}
              href={l.href}
              className="rounded-full px-3 py-1 text-[13px] font-medium text-zinc-400 transition-colors hover:bg-white/[0.06] hover:text-white"
            >
              {l.label}
            </a>
          ))}
        </div>

        {/* Right Actions */}
        <div className="flex items-center gap-2">
          <button
            onClick={() => setPaletteOpen(true)}
            className="hidden items-center gap-2 rounded-lg border border-white/10 bg-white/[0.03] px-2.5 py-1.5 text-xs text-zinc-400 transition-colors hover:border-white/20 hover:text-white md:inline-flex"
            aria-label="Open command palette"
          >
            <Search className="h-3.5 w-3.5" />
            <span>Search</span>
            <kbd className="rounded border border-white/15 bg-white/[0.06] px-1 font-mono text-[10px] text-zinc-300">
              ⌘K
            </kbd>
          </button>
          {onOpenRegistry && (
            <button
              onClick={onOpenRegistry}
              className="hidden items-center gap-1.5 rounded-lg border border-white/10 bg-white/[0.03] px-3 py-1.5 text-[13px] font-medium text-zinc-300 transition-colors hover:border-white/20 hover:bg-white/[0.06] hover:text-white md:inline-flex"
            >
              <Package className="h-3.5 w-3.5 text-[#a78bfa]" />
              Registry
            </button>
          )}

          {isLoaded && isSignedIn ? (
            <>
              <div className="hidden xl:flex items-center">
                <OrganizationSwitcher
                  hidePersonal={false}
                  afterSelectOrganizationUrl={`${import.meta.env.BASE_URL}account.html`}
                  afterSelectPersonalUrl={`${import.meta.env.BASE_URL}account.html`}
                />
              </div>
              <a
                href={`${import.meta.env.BASE_URL}account.html`}
                className="clerk-btn-primary hidden items-center gap-1.5 rounded-lg px-3.5 py-1.5 text-[13px] font-medium transition-all sm:inline-flex"
              >
                <Sparkles className="h-3.5 w-3.5" />
                Studio
              </a>
              <div className="flex items-center pl-0.5">
                <UserButton />
              </div>
            </>
          ) : (
            <>
              <SignInButton mode="modal" fallbackRedirectUrl={`${import.meta.env.BASE_URL}account.html`}>
                <button className="hidden rounded-lg px-3 py-1.5 text-[13px] font-medium text-zinc-300 transition-colors hover:text-white sm:inline-flex">
                  Sign in
                </button>
              </SignInButton>
              <SignUpButton mode="modal" fallbackRedirectUrl={`${import.meta.env.BASE_URL}account.html`}>
                <button className="clerk-btn-primary group hidden items-center gap-1 rounded-lg px-3.5 py-1.5 text-[13px] font-medium transition-all sm:inline-flex">
                  Sign up
                  <ArrowUpRight className="h-3.5 w-3.5 transition-transform group-hover:translate-x-0.5 group-hover:-translate-y-0.5" />
                </button>
              </SignUpButton>
            </>
          )}

          <button
            onClick={() => setOpen(!open)}
            className="grid h-9 w-9 place-items-center rounded-lg border border-white/10 bg-white/[0.03] text-zinc-300 lg:hidden"
            aria-label="menu"
          >
            {open ? <X className="h-4 w-4" /> : <Menu className="h-4 w-4" />}
          </button>
        </div>
      </nav>

      {open && (
        <div className="border-t border-white/[0.08] bg-[#09090b]/95 px-5 py-4 backdrop-blur-xl lg:hidden">
          <div className="space-y-1">
            {LINKS.map((l) => (
              <a
                key={l.href}
                href={l.href}
                onClick={() => setOpen(false)}
                className="block rounded-lg px-3 py-2 text-sm font-medium text-zinc-300 hover:bg-white/5 hover:text-white"
              >
                {l.label}
              </a>
            ))}
          </div>
          <div className="mt-3 flex flex-col gap-2 border-t border-white/[0.08] pt-3">
            {isLoaded && isSignedIn ? (
              <a
                href={`${import.meta.env.BASE_URL}account.html`}
                className="clerk-btn-primary rounded-lg px-4 py-2.5 text-center text-sm font-medium"
              >
                Open Publisher Studio
              </a>
            ) : (
              <button
                onClick={openAuth}
                className="clerk-btn-primary rounded-lg px-4 py-2.5 text-center text-sm font-medium"
              >
                Sign in / Publish
              </button>
            )}
          </div>
        </div>
      )}
      <CommandPalette isOpen={paletteOpen} onClose={() => setPaletteOpen(false)} />
    </motion.header>
  );
}
