import { useEffect, useState } from "react";
import { motion } from "framer-motion";
import { Menu, X, ArrowUpRight, Package, Key, User } from "lucide-react";
import { SignedIn, SignedOut, SignInButton, UserButton } from "@clerk/clerk-react";
import { EASE } from "../lib/ui";

const LINKS = [
  { label: "Academy 🎓", href: `${import.meta.env.BASE_URL}academy.html` },
  { label: "Packages 📦", href: `${import.meta.env.BASE_URL}packages.html` },
  { label: "Language", href: "#language" },
  { label: "Memory", href: "#memory" },
  { label: "Syntax", href: "#syntax" },
  { label: "Speed", href: "#performance" },
  { label: "Stdlib", href: "#stdlib" },
  { label: "Roadmap", href: "#roadmap" },
];

interface NavProps {
  onOpenRegistry?: () => void;
  onOpenAuth?: () => void;
}

export default function Nav({ onOpenRegistry, onOpenAuth }: NavProps) {
  const [scrolled, setScrolled] = useState(false);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    const fn = () => setScrolled(window.scrollY > 24);
    fn();
    window.addEventListener("scroll", fn, { passive: true });
    return () => window.removeEventListener("scroll", fn);
  }, []);

  return (
    <motion.header
      initial={{ y: -80, opacity: 0 }}
      animate={{ y: 0, opacity: 1 }}
      transition={{ duration: 0.9, ease: EASE, delay: 0.15 }}
      className={`fixed inset-x-0 top-0 z-50 transition-all duration-500 ${
        scrolled
          ? "border-b border-white/[0.07] bg-ink/80 backdrop-blur-xl"
          : "border-b border-transparent bg-transparent"
      }`}
    >
      <nav className="mx-auto flex h-[72px] max-w-7xl items-center justify-between px-5 md:px-8">
        <a href="#top" className="group flex items-center gap-3">
          <img
            src={`${import.meta.env.BASE_URL}lpp-icon-16.svg`}
            alt="L++"
            className="h-9 w-9 rounded-lg transition-transform duration-300 group-hover:-rotate-6"
          />
          <span className="hidden font-mono text-[11px] leading-tight text-white/40 sm:block">
            hybrid memory
            <br />
            language
          </span>
        </a>

        <div className="hidden items-center gap-6 lg:flex">
          {LINKS.map((l) => (
            <a
              key={l.href}
              href={l.href}
              className="font-mono text-[12px] uppercase tracking-[0.14em] text-white/55 transition-colors hover:text-acid"
            >
              {l.label}
            </a>
          ))}
        </div>

        <div className="flex items-center gap-2.5">
          {/* Clerk Authenticated State */}
          <SignedIn>
            <a
              href="/account.html"
              className="flex items-center gap-1.5 rounded-lg border border-acid/40 bg-acid/10 px-3 py-1.5 font-mono text-[12px] text-acid transition-all hover:bg-acid/20"
            >
              <Key className="h-3.5 w-3.5" />
              Developer Account
            </a>
            <UserButton
              afterSignOutUrl="/"
              appearance={{
                elements: {
                  avatarBox: "h-8 w-8 rounded-lg border border-acid/40",
                },
              }}
            />
          </SignedIn>

          {/* Clerk Signed Out State */}
          <SignedOut>
            <SignInButton mode="modal">
              <button className="flex items-center gap-1.5 rounded-lg border border-white/15 bg-white/5 px-3.5 py-1.5 font-mono text-[12px] text-white/90 transition-all hover:border-acid/40 hover:text-acid">
                <Key className="h-3.5 w-3.5 text-acid" />
                Sign In
              </button>
            </SignInButton>
          </SignedOut>

          <a
            href="#install"
            className="group hidden items-center gap-1.5 rounded-lg bg-acid px-4 py-2 font-mono text-[12px] font-semibold text-ink transition-all hover:brightness-110 sm:flex"
          >
            lpp -v
            <ArrowUpRight className="h-3.5 w-3.5 transition-transform group-hover:translate-x-0.5 group-hover:-translate-y-0.5" />
          </a>
          <button
            onClick={() => setOpen(!open)}
            className="grid h-9 w-9 place-items-center rounded-lg border border-white/10 text-white/70 lg:hidden"
            aria-label="menu"
          >
            {open ? <X className="h-4 w-4" /> : <Menu className="h-4 w-4" />}
          </button>
        </div>
      </nav>

      {open && (
        <div className="border-t border-white/[0.07] bg-ink/95 px-5 py-4 backdrop-blur-xl lg:hidden">
          {LINKS.map((l) => (
            <a
              key={l.href}
              href={l.href}
              onClick={() => setOpen(false)}
              className="block py-2.5 font-mono text-sm uppercase tracking-[0.14em] text-white/70 hover:text-acid"
            >
              {l.label}
            </a>
          ))}
          <a
            href="/packages.html"
            onClick={() => setOpen(false)}
            className="flex w-full items-center gap-2 py-2.5 font-mono text-sm uppercase tracking-[0.14em] text-acid hover:brightness-125"
          >
            <Package className="h-4 w-4" />
            Browse Packages
          </a>
          <a
            href="/account.html"
            onClick={() => setOpen(false)}
            className="flex w-full items-center gap-2 py-2.5 font-mono text-sm uppercase tracking-[0.14em] text-white/90 hover:text-acid"
          >
            <Key className="h-4 w-4 text-acid" />
            Developer Portal & Tokens
          </a>
          <a
            href="#install"
            onClick={() => setOpen(false)}
            className="mt-2 block rounded-lg bg-acid px-4 py-2.5 text-center font-mono text-sm font-semibold text-ink"
          >
            Install L++
          </a>
        </div>
      )}
    </motion.header>
  );
}
