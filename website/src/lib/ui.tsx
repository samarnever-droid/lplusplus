import { motion } from "framer-motion";
import type { ReactNode } from "react";

export const EASE: [number, number, number, number] = [0.22, 1, 0.36, 1];

export function Reveal({
  children,
  delay = 0,
  className = "",
  y = 22,
}: {
  children: ReactNode;
  delay?: number;
  className?: string;
  y?: number;
}) {
  return (
    <motion.div
      initial={{ opacity: 0, y }}
      whileInView={{ opacity: 1, y: 0 }}
      viewport={{ once: true, margin: "-60px" }}
      transition={{ duration: 0.7, delay, ease: EASE }}
      className={className}
    >
      {children}
    </motion.div>
  );
}

export function SectionHead({
  index,
  kicker,
  title,
  desc,
}: {
  index: string;
  kicker: string;
  title: ReactNode;
  desc?: ReactNode;
}) {
  return (
    <Reveal className="max-w-3xl">
      <div className="mb-5 inline-flex items-center gap-2.5 rounded-full border border-[#6c47ff]/30 bg-[#6c47ff]/10 px-3.5 py-1">
        <span className="font-mono text-[11px] font-semibold text-[#a78bfa]">{index}</span>
        <span className="h-3 w-px bg-[#6c47ff]/40" />
        <span className="text-[12px] font-medium tracking-wide text-zinc-200">
          {kicker}
        </span>
      </div>
      <h2 className="font-display text-3xl font-semibold leading-[1.08] tracking-[-0.03em] text-white sm:text-4xl md:text-[3.15rem]">
        {title}
      </h2>
      {desc && <p className="mt-4 text-base leading-relaxed text-zinc-400 sm:text-lg">{desc}</p>}
    </Reveal>
  );
}
