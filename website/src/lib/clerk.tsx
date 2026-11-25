import type { ReactNode } from "react";
import { ClerkProvider } from "@clerk/react";

const LIVE_PUBLISHABLE_KEY = "pk_live_Y2xlcmsubHBsdXNwbHVzLmJvbmQk";
const DEV_PUBLISHABLE_KEY =
  "pk_test_aGVyb2ljLWdvYmJsZXItODgxMC5jbGVyay5hY2NvdW50cy5kZXYk";

export function getClerkPublishableKey(): string {
  const fromEnv = import.meta.env.VITE_CLERK_PUBLISHABLE_KEY;
  if (typeof fromEnv === "string" && fromEnv.trim().length > 0) {
    return fromEnv.trim();
  }
  if (typeof window !== "undefined") {
    const host = window.location.hostname;
    if (host === "lplusplus.bond" || host === "www.lplusplus.bond") {
      return LIVE_PUBLISHABLE_KEY;
    }
  }
  return DEV_PUBLISHABLE_KEY;
}

export const CLERK_APPEARANCE = {
  variables: {
    colorPrimary: "#6c47ff",
    colorTextOnPrimaryBackground: "#ffffff",
    colorBackground: "#111116",
    colorInputBackground: "#09090b",
    colorInputText: "#f4f4f5",
    colorText: "#f4f4f5",
    colorTextSecondary: "#a1a1aa",
    colorNeutral: "#e4e4e7",
    borderRadius: "0.75rem",
    fontFamily: "Inter, ui-sans-serif, system-ui, sans-serif",
  },
  elements: {
    card: "bg-[#111116] border border-white/10 shadow-2xl",
    headerTitle: "text-white font-semibold tracking-tight",
    headerSubtitle: "text-zinc-400",
    socialButtonsBlockButton:
      "bg-white/[0.04] border border-white/10 text-white hover:bg-white/[0.08]",
    formFieldInput:
      "bg-[#09090b] border border-white/15 text-white focus:border-[#6c47ff]",
    formButtonPrimary:
      "bg-[#6c47ff] text-white font-medium hover:bg-[#7c5cff] shadow-lg shadow-[#6c47ff]/25",
    footerActionLink: "text-[#a78bfa] hover:text-white",
    userButtonPopoverCard: "bg-[#111116] border border-white/10",
    userButtonPopoverActionButton: "text-zinc-200 hover:bg-white/5",
    userButtonPopoverActionButtonText: "text-zinc-200",
    organizationSwitcherTrigger:
      "border border-white/10 bg-white/[0.04] text-zinc-200 hover:bg-white/[0.08] rounded-lg px-2.5 py-1.5",
  },
};

export function LppClerkProvider({ children }: { children: ReactNode }) {
  return (
    <ClerkProvider
      publishableKey={getClerkPublishableKey()}
      appearance={CLERK_APPEARANCE}
      afterSignOutUrl="/"
    >
      {children}
    </ClerkProvider>
  );
}
