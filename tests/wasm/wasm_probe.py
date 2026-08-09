#!/usr/bin/env python3
"""wasm_probe.py — zero-dependency WebAssembly module inspector for CI logs.

Prints a compact structural dump (sections, type entries, function-section
map, table, elem seats, exports, locals-per-body counts) so failures in
tests/run_wasm_tests.sh can be diagnosed from the Actions log alone.

Usage: wasm_probe.py <module.wasm> [--window OFFSET SPAN]
"""
import sys

VALNAMES = {0x7F: "i32", 0x7E: "i64", 0x7D: "f32", 0x7C: "f64", 0x7B: "v128", 0x70: "funcref", 0x6F: "externref"}
SEC = {0: "custom", 1: "type", 2: "import", 3: "function", 4: "table", 5: "memory", 6: "global", 7: "export",
      8: "start", 9: "elem", 10: "code", 11: "data", 12: "datacount"}


class R:
    def __init__(self, b):
        self.b = b
        self.p = 0

    def u8(self):
        v = self.b[self.p]
        self.p += 1
        return v

    def uleb(self):
        r = 0
        s = 0
        while True:
            x = self.u8()
            r |= (x & 0x7F) << s
            if not x & 0x80:
                return r
            s += 7

    def name(self):
        n = self.uleb()
        s = self.b[self.p:self.p + n].decode("utf-8", "replace")
        self.p += n
        return s


def vname(byte):
    return VALNAMES.get(byte, f"0x{byte:02x}?")


def main():
    path = sys.argv[1]
    b = open(path, "rb").read()
    print(f"module {path}: {len(b)} bytes")
    if b[:8] != b"\x00asm\x01\x00\x00\x00":
        print("BAD MAGIC HEADER")
        return 1
    types, fns, elems, nimports = [], [], [], 0
    r = R(b)
    r.p = 8
    while r.p < len(b):
        sid = r.u8()
        size = r.uleb()
        start = r.p
        end = start + size
        print(f"section {SEC.get(sid, sid)} (id {sid}): offset {start}, size {size}")
        s = R(b)
        s.p = start
        try:
            if sid == 1:
                n = s.uleb()
                for i in range(n):
                    form = s.u8()
                    np_ = s.uleb()
                    params = [vname(s.u8()) for _ in range(np_)]
                    nr = s.uleb()
                    results = [vname(s.u8()) for _ in range(nr)]
                    types.append((params, results))
                    print(f"  type[{i}] = ({' '.join(params)}) -> ({' '.join(results)})")
            elif sid == 3:
                n = s.uleb()
                fns = [s.uleb() for _ in range(n)]
                for i, t in enumerate(fns):
                    print(f"  fn section[{i}] -> type[{t}]")
            elif sid == 2:
                n = s.uleb()
                for i in range(n):
                    mod, nm, kind = s.name(), s.name(), s.u8()
                    detail = ""
                    if kind == 0:
                        detail = f"type[{s.uleb()}]"
                    elif kind == 1:
                        lim = s.u8()
                        lo = s.uleb()
                        detail = f"table {lo}+"
                    elif kind == 2:
                        lim, lo = s.u8(), s.uleb()
                        hi = s.uleb() if lim else None
                        detail = f"memory {lo}..{hi}"
                    elif kind == 3:
                        detail = f"global {vname(s.u8())}"
                    print(f"  import {mod}.{nm} kind {kind} {detail}")
                    if kind == 0:
                        nimports += 1
            elif sid == 4:
                n = s.uleb()
                for i in range(n):
                    et = vname(s.u8())
                    lim, lo = s.u8(), s.uleb()
                    hi = s.uleb() if lim else None
                    print(f"  table[{i}] {et} min {lo} max {hi}")
            elif sid == 7:
                n = s.uleb()
                for i in range(n):
                    nm, kind, idx = s.name(), s.u8(), s.uleb()
                    print(f"  export {nm} kind {kind} idx {idx}")
            elif sid == 9:
                n = s.uleb()
                for i in range(n):
                    flag = s.uleb()
                    if flag != 0:
                        print(f"  elem[{i}] flag {flag} (not decoded)")
                        break
                    op = s.u8()
                    assert op == 0x41, hex(op)  # i32.const
                    off = 0
                    sh = 0
                    while True:
                        x = s.u8()
                        off |= (x & 0x7F) << sh
                        if not x & 0x80:
                            break
                        sh += 7
                    assert s.u8() == 0x0B  # end
                    cnt = s.uleb()
                    seats = [s.uleb() for _ in range(cnt)]
                    elems = seats
                    print(f"  elem[{i}] offset {off}, {cnt} seats: {seats}")
            elif sid == 10:
                n = s.uleb()
                print(f"  {n} code bodies")
        except Exception as e:  # keep dumping whatever parsed
            print(f"  <decode stopped: {e}>")
        # resolve seats to types once both known
        if sid == 9 and elems and fns:
            pass
        # advance to the next section regardless of sub-parse
        r.p = end
    if fns and types and elems:
        print("seat map (seat -> fn -> type):")
        for seat, fnidx in enumerate(elems):
            tidx = fns[fnidx - nimports] if 0 <= fnidx - nimports < len(fns) else None
            txt = "?"
            if tidx is not None and tidx < len(types):
                params, results = types[tidx]
                txt = f"({' '.join(params)})->({' '.join(results)})"
            print(f"  seat {seat}: fn {fnidx}, type[{tidx}] {txt}")
    if "--window" in sys.argv:
        i = sys.argv.index("--window")
        off, span = int(sys.argv[i + 1]), int(sys.argv[i + 2])
        lo = max(0, off - span // 2)
        chunk = b[lo:off + span // 2]
        for row in range(0, len(chunk), 16):
            piece = chunk[row:row + 16]
            hexs = " ".join(f"{c:02x}" for c in piece)
            print(f"  {lo + row:08x}  {hexs}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
