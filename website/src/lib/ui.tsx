import { motion } from "framer-motion";
import type { ReactNode } from "react";

export const EASE: [number, number, number, number] = [0.22, 1, 0.36, 1];

export function Reveal({
  children,
  delay = 0,
  className = "",
  y = 28,
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
      viewport={{ once: true, margin: "-70px" }}
      transition={{ duration: 0.8, delay, ease: EASE }}
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
      <div className="mb-5 flex items-center gap-3">
        <span className="font-mono text-xs text-acid">[{index}]</span>
        <span className="h-px w-10 bg-acid/40" />
        <span className="font-mono text-[11px] uppercase tracking-[0.28em] text-white/45">
          {kicker}
        </span>
      </div>
      <h2 className="font-display text-4xl font-semibold leading-[1.04] tracking-tight text-white md:text-[3.4rem]">
        {title}
      </h2>
      {desc && <p className="mt-5 text-lg leading-relaxed text-white/55">{desc}</p>}
    </Reveal>
  );
}
