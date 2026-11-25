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
    colorPrimary: "#c8ff2e",
    colorTextOnPrimaryBackground: "#07090d",
    colorBackground: "#0b0f17",
    colorInputBackground: "#05070a",
    colorInputText: "#ffffff",
    colorText: "#ffffff",
    colorTextSecondary: "#9ca3af",
    colorNeutral: "#e5e7eb",
    borderRadius: "0.85rem",
    fontFamily: "Inter, ui-sans-serif, system-ui, sans-serif",
  },
  elements: {
    card: "bg-[#0b0f17] border border-white/10 shadow-2xl",
    headerTitle: "text-white font-mono",
    headerSubtitle: "text-white/60",
    socialButtonsBlockButton:
      "bg-white/5 border border-white/10 text-white hover:bg-white/10",
    formFieldInput:
      "bg-[#05070a] border border-white/15 text-white focus:border-[#c8ff2e]",
    formButtonPrimary:
      "bg-[#c8ff2e] text-[#07090d] font-mono font-bold hover:brightness-110",
    footerActionLink: "text-[#c8ff2e] hover:brightness-110",
    userButtonPopoverCard: "bg-[#0b0f17] border border-white/10",
    userButtonPopoverActionButton: "text-white/80 hover:bg-white/5",
    userButtonPopoverActionButtonText: "text-white/85",
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
