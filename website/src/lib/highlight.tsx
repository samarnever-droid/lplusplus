import { useMemo } from "react";

/* ------------------------------------------------------------------ */
/*  L++ tokenizer — hand-rolled, zero deps                             */
/* ------------------------------------------------------------------ */

const KEYWORDS = new Set([
  "def", "return", "mut", "struct", "fn", "spawn", "if", "else",
  "while", "import", "for", "in", "not", "and", "or", "break", "continue",
]);
const BOOLEANS = new Set(["true", "false", "True", "False"]);
const BUILTINS = new Set([
  "print", "print_str", "input", "read_file", "write_file",
  "list_new", "list_push", "list_get", "list_len", "list_free",
  "json_parse", "json_get_int", "json_get_str", "json_get_obj", "json_free",
  "map", "range", "len",
]);

const C = {
  kw: "text-acid",
  type: "text-lav",
  str: "text-ember",
  com: "text-[#565d6b] italic",
  num: "text-aqua",
  bi: "text-[#7cc7ff]",
  call: "text-[#eef0f4]",
  op: "text-[#9aa0ac]",
  id: "text-[#c9cdd6]",
  pn: "text-[#6b7079]",
};

export interface Tok {
  text: string;
  cls: string;
}

const MASTER =
  /(#.*$)|("(?:[^"\\]|\\.)*")|(\d[\d_]*)|(:=|->|==|!=|<=|>=|&&|\|\||[+\-*/%=<>])|([A-Za-z_][A-Za-z0-9_]*)|(\s+)|(.)/gm;

export function tokenizeLine(src: string): Tok[] {
  const toks: Tok[] = [];
  MASTER.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = MASTER.exec(src)) !== null) {
    const [full, com, str, num, op, ident, ws] = m;
    if (com !== undefined) toks.push({ text: com, cls: C.com });
    else if (str !== undefined) toks.push({ text: str, cls: C.str });
    else if (num !== undefined) toks.push({ text: num, cls: C.num });
    else if (op !== undefined) toks.push({ text: op, cls: C.op });
    else if (ident !== undefined) {
      if (KEYWORDS.has(ident)) toks.push({ text: ident, cls: C.kw });
      else if (BOOLEANS.has(ident)) toks.push({ text: ident, cls: C.num });
      else if (BUILTINS.has(ident)) toks.push({ text: ident, cls: C.bi });
      else if (/^[A-Z]/.test(ident)) toks.push({ text: ident, cls: C.type });
      else {
        // peek ahead: function call?
        const rest = src.slice(MASTER.lastIndex);
        toks.push({ text: ident, cls: /^\s*\(/.test(rest) ? C.call : C.id });
      }
    } else if (ws !== undefined) toks.push({ text: ws, cls: "" });
    else toks.push({ text: full, cls: C.pn });
  }
  return toks;
}

/* ------------------------------------------------------------------ */
/*  <Code/> — renders tokenized source                                 */
/* ------------------------------------------------------------------ */

export function Code({ src }: { src: string }) {
  const lines = useMemo(() => src.split("\n").map(tokenizeLine), [src]);
  return (
    <>
      {lines.map((toks, i) => (
        <span key={i} className="block">
          {toks.map((t, j) => (
            <span key={j} className={t.cls}>
              {t.text}
            </span>
          ))}
          {toks.length === 0 ? " " : ""}
        </span>
      ))}
    </>
  );
}

/* ------------------------------------------------------------------ */
/*  <CodeBlock/> — window chrome + optional line highlighting          */
/* ------------------------------------------------------------------ */

interface CodeBlockProps {
  code: string;
  title?: string;
  highlight?: number[]; // 1-indexed lines
  lineNumbers?: boolean;
  className?: string;
  badge?: string;
}

export function CodeBlock({
  code,
  title = "main.lpp",
  highlight,
  lineNumbers = true,
  className = "",
  badge,
}: CodeBlockProps) {
  const lines = useMemo(() => code.split("\n"), [code]);
  return (
    <div
      className={`overflow-hidden rounded-2xl border border-white/10 bg-panel shadow-[0_20px_60px_-20px_rgb(0_0_0/0.8)] ${className}`}
    >
      <div className="flex items-center gap-2 border-b border-white/[0.07] bg-white/[0.025] px-4 py-3">
        <span className="h-2.5 w-2.5 rounded-full bg-[#ff5f57]" />
        <span className="h-2.5 w-2.5 rounded-full bg-[#febc2e]" />
        <span className="h-2.5 w-2.5 rounded-full bg-[#28c840]" />
        <span className="ml-3 font-mono text-[11px] tracking-wide text-white/40">{title}</span>
        {badge && (
          <span className="ml-auto rounded-md border border-acid/25 bg-acid/10 px-2 py-0.5 font-mono text-[10px] font-medium text-acid">
            {badge}
          </span>
        )}
      </div>
      <pre className="code-scroll overflow-x-auto p-5 font-mono text-[12.5px] leading-[1.75] md:text-[13px]">
        {lines.map((ln, i) => {
          const n = i + 1;
          const hot = highlight?.includes(n);
          const dim = highlight && !hot;
          return (
            <div
              key={i}
              className={`flex rounded-md transition-colors duration-300 ${
                hot ? "bg-acid/[0.09] shadow-[inset_2px_0_0_0_var(--color-acid)]" : ""
              } ${dim ? "opacity-45" : ""}`}
            >
              {lineNumbers && (
                <span className="w-8 shrink-0 select-none pr-4 text-right text-white/20">{n}</span>
              )}
              <code className="whitespace-pre">
                <Code src={ln} />
              </code>
            </div>
          );
        })}
      </pre>
    </div>
  );
}
