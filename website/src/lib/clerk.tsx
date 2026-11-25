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
    colorPrimary: "#7c5cff",
    colorTextOnPrimaryBackground: "#ffffff",
    colorBackground: "#181820",
    colorInputBackground: "#23232f",
    colorInputText: "#ffffff",
    colorText: "#ffffff",
    colorTextSecondary: "#e4e4e7",
    colorNeutral: "#f4f4f5",
    colorDanger: "#f87171",
    colorSuccess: "#34d399",
    borderRadius: "0.75rem",
    fontFamily: "Inter, ui-sans-serif, system-ui, sans-serif",
  },
  elements: {
    card: {
      backgroundColor: "#181820",
      border: "1px solid rgba(255, 255, 255, 0.14)",
      boxShadow: "0 24px 60px rgba(0, 0, 0, 0.75)",
    },
    formFieldInput: {
      backgroundColor: "#23232f",
      color: "#ffffff",
      borderColor: "rgba(255, 255, 255, 0.18)",
    },
    formFieldLabel: {
      color: "#f4f4f5",
    },
    socialButtonsBlockButton: {
      backgroundColor: "#23232f",
      color: "#ffffff",
      borderColor: "rgba(255, 255, 255, 0.18)",
    },
    socialButtonsBlockButtonText: {
      color: "#ffffff",
    },
    headerTitle: {
      color: "#ffffff",
    },
    headerSubtitle: {
      color: "#d4d4d8",
    },
    dividerText: {
      color: "#d4d4d8",
    },
    footerActionText: {
      color: "#d4d4d8",
    },
    footerActionLink: {
      color: "#a78bfa",
    },
    userButtonPopoverCard: {
      backgroundColor: "#181820",
      border: "1px solid rgba(255, 255, 255, 0.14)",
    },
    organizationSwitcherTrigger: {
      color: "#ffffff",
      backgroundColor: "#23232f",
      border: "1px solid rgba(255, 255, 255, 0.16)",
      borderRadius: "0.5rem",
      padding: "0.35rem 0.65rem",
    },
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
