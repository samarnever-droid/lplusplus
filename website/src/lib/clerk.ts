import { dark } from "@clerk/themes";

export const CLERK_PUBLISHABLE_KEY =
  import.meta.env.VITE_CLERK_PUBLISHABLE_KEY ||
  "pk_live_Y2xlcmsubHBsdXNwbHVzLmJvbmQk";

export const clerkAppearance = {
  baseTheme: dark,
  variables: {
    colorPrimary: "#c8f14b",
    colorBackground: "#0b0e14",
    colorInputBackground: "#07090d",
    colorInputText: "#ffffff",
    colorText: "#ffffff",
    colorTextSecondary: "rgba(255, 255, 255, 0.65)",
    colorNeutral: "#ffffff",
    borderRadius: "0.85rem",
    fontFamily: '"Space Grotesk", "Inter", sans-serif',
  },
  elements: {
    card: "border border-white/10 shadow-2xl bg-[#0b0e14]/95 backdrop-blur-2xl rounded-2xl",
    headerTitle: "font-mono text-xl font-bold tracking-tight text-white",
    headerSubtitle: "text-white/60 font-sans text-xs",
    socialButtonsBlockButton:
      "border border-white/15 bg-white/[0.04] text-white hover:bg-white/10 hover:border-white/25 transition-all font-mono text-xs rounded-xl",
    formButtonPrimary:
      "bg-[#c8f14b] text-[#070809] hover:brightness-110 font-mono font-bold text-xs py-2.5 rounded-xl shadow-[0_0_20px_rgba(200,241,75,0.3)] transition-all",
    formFieldInput:
      "border border-white/15 focus:border-[#c8f14b]/70 bg-[#07090d] text-white rounded-xl text-xs py-2.5 transition-all font-mono",
    formFieldLabel: "text-white/80 font-mono text-xs uppercase tracking-wider",
    footerActionLink: "text-[#c8f14b] hover:underline font-mono text-xs font-semibold",
    identityPreviewText: "text-white font-mono text-xs",
    organizationSwitcherTrigger:
      "border border-white/15 bg-white/5 rounded-xl font-mono text-xs text-white hover:border-[#c8f14b]/40 py-2 px-3",
    userButtonAvatarBox: "h-9 w-9 rounded-xl border border-[#c8f14b]/40 shadow-md",
    userButtonPopoverCard:
      "border border-white/15 bg-[#0b0e14]/95 backdrop-blur-2xl shadow-2xl rounded-2xl",
    organizationProfileCard:
      "border border-white/15 bg-[#0b0e14]/95 backdrop-blur-2xl shadow-2xl rounded-2xl",
    userProfileCard:
      "border border-white/15 bg-[#0b0e14]/95 backdrop-blur-2xl shadow-2xl rounded-2xl",
  },
};
