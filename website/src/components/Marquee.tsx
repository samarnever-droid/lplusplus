const ITEMS = [
  "no garbage collector",
  "no borrow checker",
  "semantic escape analysis",
  "cranelift aot",
  "c transpiler",
  "arena allocation",
  "arc managed heap",
  "zero-cost stack values",
  "3 ms compiles",
  "native x86-64",
];

export default function Marquee() {
  const row = (
    <>
      {ITEMS.map((t) => (
        <span key={t} className="flex shrink-0 items-center gap-6 pr-6">
          <span className="font-mono text-[13px] uppercase tracking-[0.22em] text-white/60">
            {t}
          </span>
          <span className="font-mono text-[13px] font-bold text-acid">++</span>
        </span>
      ))}
    </>
  );
  return (
    <div className="relative border-y border-white/[0.07] bg-panel/60 py-4">
      <div className="mask-fade-x overflow-hidden">
        <div className="flex w-max animate-marquee">
          {row}
          {row}
        </div>
      </div>
    </div>
  );
}
