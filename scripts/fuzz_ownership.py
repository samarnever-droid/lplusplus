#!/usr/bin/env python3
"""Seeded differential fuzzer for L++ ownership/ARC shapes.

Generates random self-checking L++ programs from composable ownership
blocks (reference rings, managed tuples, tuple-routed cycles, list
aliasing, closure captures, overwrite churn). Every block's checksum
contribution is computed exactly in Python, so the program's expected
stdout is known by construction. A mismatch, crash, or timeout means
the compiler/runtime diverged from the modelled semantics — the
failing program is saved under fuzz_failures/ and the seed reproduces
it deterministically. Promote real failures to tests/*.lpp corpus
files once minimized.

Usage:
  scripts/fuzz_ownership.py [--seed N] [--count M] [--keep] [--timeout S]

Requires ./target/debug/lpp (build it first: cargo build -p lpp).
This fuzzes the ownership/ARC surface (the historically bug-dense
area); it is not a parser fuzzer.
"""

import argparse
import os
import random
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LPP = os.path.join(ROOT, "target", "debug", "lpp")


class Prog:
    def __init__(self, rng):
        self.rng = rng
        self.decls = []
        self.body = []
        self.expected = 0
        self.counter = 0

    def fresh(self, base):
        self.counter += 1
        return f"{base}{self.counter}"

    # ---- blocks: each appends statements and the exact checksum ----

    def ring(self):
        k = self.rng.randint(2, 5)
        reps = self.rng.choice([1, 3, 50])
        node = self.fresh("Node")
        self.decls.append(f"struct {node}:\n    v: Int\n    next: {node}\n")
        names = [f"n{i}" for i in range(k)]
        b = [f"    mut i := 0", f"    while i < {reps}:"]
        for j, nm in enumerate(names):
            b.append(f"        mut {nm} := {node}({j + 1}, 0)")
        for j, nm in enumerate(names):
            b.append(f"        {nm}.next = {names[(j + 1) % k]}")
        b.append(f"        mut cur := {names[0]}")
        b.append(f"        mut s := 0")
        b.append(f"        mut w := 0")
        b.append(f"        while w < {k}:")
        b.append(f"            s = s + cur.v")
        b.append(f"            cur = cur.next")
        b.append(f"            w = w + 1")
        b.append(f"        total = total + s")
        b.append(f"        i = i + 1")
        self.body += b
        self.expected += reps * (k * (k + 1) // 2)

    def tuple_churn(self):
        reps = self.rng.choice([10, 500, 2000])
        box = self.fresh("Box")
        self.decls.append(f"struct {box}:\n    v: Int\n")
        self.body += [
            "    mut i := 0",
            f"    while i < {reps}:",
            f'        t := ({box}(i), "s")',
            "        (b, s) := t",
            "        total = total + b.v - i + str_len(s)",
            "        i = i + 1",
        ]
        self.expected += reps  # str_len("s") == 1 per iteration

    def tuple_cycle(self):
        reps = self.rng.choice([1, 100, 1000])
        h, n = self.fresh("H"), self.fresh("N")
        self.decls.append(f"struct {h}:\n    t: ({n}, Int)\n")
        self.decls.append(f"struct {n}:\n    id: Int\n    h: {h}\n")
        self.body += [
            "    mut i := 0",
            f"    while i < {reps}:",
            f"        mut a := {n}(1, 0)",
            f"        mut b := {n}(2, 0)",
            f"        a.h = {h}((b, 10))",
            "        mut bb := a.h.t.0",
            f"        bb.h = {h}((a, 20))",
            "        total = total + bb.h.t.0.id + bb.id",
            "        i = i + 1",
        ]
        self.expected += 3 * reps

    def list_alias(self):
        vals = [self.rng.randint(1, 9) for _ in range(3)]
        idx = self.rng.randint(0, 2)
        box = self.fresh("Item")
        self.decls.append(f"struct {box}:\n    v: Int\n")
        items = ", ".join(f"{box}({v})" for v in vals)
        self.body += [
            f"    items := [{items}]",
            f"    picked := list_get(items, {idx})",
            "    total = total + picked.v",
        ]
        self.expected += vals[idx]

    def closure_tuple(self):
        reps = self.rng.choice([10, 300])
        base = self.rng.randint(1, 50)
        self.body += [
            "    mut i := 0",
            f"    while i < {reps}:",
            f'        base := ({base}, "c")',
            "        f := fn() -> Int:",
            "            return base.0 + 7",
            "        v := f()",
            "        total = total + v",
            "        i = i + 1",
        ]
        self.expected += (base + 7) * reps

    def overwrite(self):
        reps = self.rng.choice([10, 1000])
        box, holder = self.fresh("Cell"), self.fresh("Holder")
        self.decls.append(f"struct {box}:\n    v: Int\n")
        self.decls.append(f"struct {holder}:\n    pair: ({box}, Int)\n")
        self.body += [
            f"    mut h := {holder}(({box}(0), 0))",
            "    mut i := 0",
            f"    while i < {reps}:",
            f"        h.pair = ({box}(i), i)",
            "        i = i + 1",
            "    total = total + h.pair.0.v",
        ]
        self.expected += reps - 1

    BLOCKS = ["ring", "tuple_churn", "tuple_cycle", "list_alias",
              "closure_tuple", "overwrite"]

    def render(self):
        blocks = self.rng.randint(1, 4)
        for _ in range(blocks):
            getattr(self, self.rng.choice(self.BLOCKS))()
        src = "\n".join(self.decls)
        src += "\ndef main():\n    mut total := 0\n"
        src += "\n".join(self.body)
        src += "\n    print(total)\n"
        return src, f"{self.expected}\n"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--count", type=int, default=50)
    ap.add_argument("--timeout", type=int, default=30)
    ap.add_argument("--keep", action="store_true",
                    help="keep generated programs in the temp dir")
    args = ap.parse_args()

    if not os.path.isfile(LPP):
        print(f"error: {LPP} not found — run: cargo build -p lpp", file=sys.stderr)
        return 2

    failures = 0
    fail_dir = os.path.join(ROOT, "fuzz_failures")
    tmp_root = tempfile.mkdtemp(prefix="lpp_fuzz_")
    for case in range(args.count):
        seed = args.seed * 100_000 + case
        rng = random.Random(seed)
        source, expected = Prog(rng).render()
        work = os.path.join(tmp_root, f"case_{case}")
        os.makedirs(work, exist_ok=True)
        path = os.path.join(work, "prog.lpp")
        with open(path, "w") as fh:
            fh.write(source)
        try:
            out = subprocess.run([LPP, "run", path], capture_output=True,
                                 text=True, timeout=args.timeout, cwd=work)
            ok = out.returncode == 0 and out.stdout == expected
            detail = (f"exit={out.returncode} stdout={out.stdout!r} "
                      f"expected={expected!r} stderr={out.stderr[:400]!r}")
        except subprocess.TimeoutExpired:
            ok, detail = False, f"timeout after {args.timeout}s"
        if not ok:
            failures += 1
            os.makedirs(fail_dir, exist_ok=True)
            saved = os.path.join(fail_dir, f"seed{seed}.lpp")
            with open(saved, "w") as fh:
                fh.write(source)
            print(f"FAIL case={case} seed={seed} saved={saved}\n  {detail}")
            break
    if failures == 0:
        print(f"fuzz_ownership: {args.count}/{args.count} PASS "
              f"(seed base {args.seed}) in {tmp_root}")
    if not args.keep and failures == 0:
        import shutil
        shutil.rmtree(tmp_root, ignore_errors=True)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
