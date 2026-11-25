import type { ReactNode } from "react";
import { ClerkProvider } from "@clerk/react";
import { dark } from "@clerk/themes";

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
  baseTheme: dark,
  variables: {
    colorPrimary: "#6c47ff",
    colorTextOnPrimaryBackground: "#ffffff",
    colorBackground: "#111116",
    colorInputBackground: "#09090b",
    colorInputText: "#ffffff",
    colorText: "#ffffff",
    colorTextSecondary: "#d4d4d8",
    colorNeutral: "#e4e4e7",
    colorDanger: "#f87171",
    colorSuccess: "#34d399",
    borderRadius: "0.75rem",
    fontFamily: "Inter, ui-sans-serif, system-ui, sans-serif",
  },
  elements: {
    card: "bg-[#111116] border border-white/15 shadow-2xl",
    headerTitle: "text-white font-semibold tracking-tight",
    headerSubtitle: "text-zinc-300",
    dividerLine: "bg-white/15",
    dividerText: "text-zinc-400",
    socialButtonsBlockButton:
      "bg-white/[0.06] border border-white/15 text-white hover:bg-white/[0.12]",
    socialButtonsBlockButtonText: "text-white font-medium",
    formFieldLabel: "text-zinc-200 font-medium",
    formFieldInput:
      "bg-[#09090b] border border-white/20 text-white placeholder:text-zinc-500 focus:border-[#6c47ff]",
    formButtonPrimary:
      "bg-[#6c47ff] text-white font-semibold hover:bg-[#7c5cff] shadow-lg shadow-[#6c47ff]/30",
    footer: "bg-[#0d0d12] border-t border-white/10",
    footerActionText: "text-zinc-300",
    footerActionLink: "text-[#a78bfa] font-medium hover:text-white",
    identityPreviewText: "text-white",
    identityPreviewEditButton: "text-[#a78bfa]",
    userButtonPopoverCard: "bg-[#111116] border border-white/15 shadow-2xl",
    userButtonPopoverActionButton: "text-zinc-100 hover:bg-white/10",
    userButtonPopoverActionButtonText: "text-zinc-100",
    userButtonPopoverActionButtonIcon: "text-zinc-300",
    userButtonPopoverFooter: "bg-[#0d0d12] border-t border-white/10",
    organizationSwitcherTrigger:
      "border border-white/15 bg-white/[0.06] text-white hover:bg-white/[0.12] rounded-lg px-2.5 py-1.5",
    organizationSwitcherPopoverCard:
      "bg-[#111116] border border-white/15 shadow-2xl",
    navbarButton: "text-zinc-200 hover:text-white hover:bg-white/5",
    pageScrollBox: "bg-[#111116]",
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
