#!/usr/bin/env python3
"""Reliable, repeatable cross-language benchmark harness for L++.

This is a correctness-first comparison: each implementation must produce the
same stdout before timing is recorded. Optional toolchains are reported as SKIP,
never silently omitted. Build and execution timeouts prevent a broken compiler
or benchmark from stalling CI.
"""
from __future__ import annotations

import argparse
import json
import os
import platform
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
TIMEOUT_SECONDS = 60

# Every entry uses the same algorithm and expected stdout. More workloads can be
# added only with equivalent implementations and a correctness oracle.
WORKLOADS = {
    "fib35": {
        "expected": "9227465",
        "lpp": ROOT / "benchmarks/bench_fib.lpp",
        "body": {
            "c": '#include <stdio.h>\nlong long f(long long n){return n<2?n:f(n-1)+f(n-2);}int main(){printf("%lld\\n",f(35));}\n',
            "cpp": '#include <cstdio>\nlong long f(long long n){return n<2?n:f(n-1)+f(n-2);}int main(){std::printf("%lld\\n",f(35));}\n',
            "rust": 'fn f(n:i64)->i64{if n<2{n}else{f(n-1)+f(n-2)}}fn main(){println!("{}",f(35));}\n',
            "go": 'package main\nimport "fmt"\nfunc f(n int64)int64{if n<2{return n};return f(n-1)+f(n-2)}func main(){fmt.Println(f(35))}\n',
            "zig": 'const std=@import("std");fn f(n:i64)i64{return if(n<2)n else f(n-1)+f(n-2);}pub fn main()!void{std.debug.print("{}\\n",.{f(35)});}\n',
            "java": 'public class Main{static long f(long n){return n<2?n:f(n-1)+f(n-2);}public static void main(String[]a){System.out.println(f(35));}}\n',
            "python": 'def f(n): return n if n < 2 else f(n-1)+f(n-2)\nprint(f(35))\n',
            "node": 'function f(n){return n<2?n:f(n-1)+f(n-2)} console.log(f(35))\n',
            "ruby": 'def f(n); n<2 ? n : f(n-1)+f(n-2); end\nputs f(35)\n',
        },
    },
    "loop10m": {
        "expected": "49999995000000",
        "lpp": ROOT / "benchmarks/bench_loop.lpp",
        "body": {
            "c": '#include <stdio.h>\nint main(){long long a=0;for(long long i=0;i<10000000;i++)a+=i;printf("%lld\\n",a);}\n',
            "cpp": '#include <cstdio>\nint main(){long long a=0;for(long long i=0;i<10000000;i++)a+=i;std::printf("%lld\\n",a);}\n',
            "rust": 'fn main(){let mut a:i64=0;for i in 0..10_000_000i64{a+=i;}println!("{}",a)}\n',
            "go": 'package main\nimport "fmt"\nfunc main(){var a int64;for i:=int64(0);i<10000000;i++{a+=i};fmt.Println(a)}\n',
            "zig": 'const std=@import("std");pub fn main()!void{var a:i64=0;var i:i64=0;while(i<10000000):(i+=1){a+=i;}std.debug.print("{}\\n",.{a});}\n',
            "java": 'public class Main{public static void main(String[]a){long x=0;for(long i=0;i<10000000;i++)x+=i;System.out.println(x);}}\n',
            "python": 'a=0\nfor i in range(10000000): a+=i\nprint(a)\n',
            "node": 'let a=0n;for(let i=0n;i<10000000n;i++)a+=i;console.log(a.toString())\n',
            "ruby": 'a=0\n10_000_000.times{|i|a+=i}\nputs a\n',
        },
    },
}

# `run` is an interpreter invocation for dynamic languages; `build` creates a
# native/VM artifact. Every language remains optional to keep local use honest.
LANGUAGES = {
    "c": {"tool": "cc", "ext": "c", "build": lambda s,e,d: [["cc", "-O3", "-DNDEBUG", str(s), "-o", str(e)]], "run": lambda e,d: [str(e)]},
    "cpp": {"tool": "c++", "ext": "cpp", "build": lambda s,e,d: [["c++", "-O3", "-DNDEBUG", str(s), "-o", str(e)]], "run": lambda e,d: [str(e)]},
    "rust": {"tool": "rustc", "ext": "rs", "build": lambda s,e,d: [["rustc", "-C", "opt-level=3", str(s), "-o", str(e)]], "run": lambda e,d: [str(e)]},
    "go": {"tool": "go", "ext": "go", "build": lambda s,e,d: [["go", "build", "-trimpath", "-o", str(e), str(s)]], "run": lambda e,d: [str(e)]},
    "zig": {"tool": "zig", "ext": "zig", "build": lambda s,e,d: [["zig", "build-exe", "-O", "ReleaseFast", str(s), "-femit-bin=" + str(e)]], "run": lambda e,d: [str(e)]},
    "java": {"tool": "javac", "ext": "java", "build": lambda s,e,d: [["javac", "-d", str(d), str(s)]], "run": lambda e,d: ["java", "-cp", str(d), "Main"]},
    "python": {"tool": "python3", "ext": "py", "build": lambda s,e,d: [], "run": lambda e,d: ["python3", str(e)]},
    "node": {"tool": "node", "ext": "js", "build": lambda s,e,d: [], "run": lambda e,d: ["node", str(e)]},
    "ruby": {"tool": "ruby", "ext": "rb", "build": lambda s,e,d: [], "run": lambda e,d: ["ruby", str(e)]},
}

def command_version(tool: str) -> str:
    for flag in ("--version", "-version", "-v"):
        try:
            return subprocess.check_output([tool, flag], text=True, stderr=subprocess.STDOUT, timeout=10).splitlines()[0]
        except Exception:
            pass
    return "unavailable"

def invoke(cmd: list[str], cwd: Path, timeout: int = TIMEOUT_SECONDS) -> tuple[subprocess.CompletedProcess[str], float]:
    started = time.perf_counter()
    try:
        result = subprocess.run(cmd, cwd=cwd, text=True, capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired as exc:
        result = subprocess.CompletedProcess(cmd, 124, exc.stdout or "", (exc.stderr or "") + "\nTIMEOUT")
    return result, (time.perf_counter() - started) * 1000

def stats(samples: list[float]) -> dict[str, float]:
    ordered = sorted(samples)
    return {"median_ms": round(statistics.median(ordered), 3), "min_ms": round(ordered[0], 3), "max_ms": round(ordered[-1], 3), "p95_ms": round(ordered[min(len(ordered)-1, round((len(ordered)-1)*.95))], 3)}

def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repetitions", type=int, default=5)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--timeout", type=int, default=TIMEOUT_SECONDS)
    args = parser.parse_args()
    if args.repetitions < 1 or args.warmups < 0: raise SystemExit("invalid repetition/warmup count")
    if not shutil.which("cargo") or not shutil.which("cc"): raise SystemExit("requires cargo and cc for L++")
    subprocess.run(["cargo", "build", "--release", "--locked", "--bin", "lpp"], cwd=ROOT, check=True)
    lpp = ROOT / "target/release" / ("lpp.exe" if os.name == "nt" else "lpp")
    rows = []
    with tempfile.TemporaryDirectory(prefix="lpp-compare-") as raw:
        temp = Path(raw)
        for workload, spec in WORKLOADS.items():
            # Two explicit L++ AOT modes: iteration latency and optimized release.
            for profile in ("none", "speed"):
                source = temp / f"{workload}-lpp-{profile}.lpp"; source.write_text(spec["lpp"].read_text())
                env = os.environ | {"LPP_AOT": "1", "LPP_AOT_OPT": profile}
                compile_run, compile_ms = invoke([str(lpp), "emit", str(source), "--aot"], temp, args.timeout)
                exe = temp / f"{workload}-lpp-{profile}"
                link_run, link_ms = invoke(["cc", "-O2", str(source.with_suffix('.o')), str(ROOT / "lpp_runtime.c"), "-o", str(exe), "-pthread"], temp, args.timeout) if compile_run.returncode == 0 else (subprocess.CompletedProcess([], 1, "", "compile failed"), 0.0)
                samples=[]; output=""
                if link_run.returncode == 0:
                    for _ in range(args.warmups): invoke([str(exe)], temp, args.timeout)
                    for _ in range(args.repetitions):
                        run, elapsed=invoke([str(exe)], temp, args.timeout); output=run.stdout.strip()
                        if run.returncode == 0 and output == spec["expected"]: samples.append(elapsed)
                rows.append({"workload":workload,"language":f"lpp-aot-{profile}","status":"PASS" if len(samples)==args.repetitions else "FAIL","compile_ms":round(compile_ms,3),"link_ms":round(link_ms,3),"build_ms":round(compile_ms+link_ms,3),"runtime":stats(samples) if samples else None,"tool":"Cranelift AOT + host link"})
            for language, cfg in LANGUAGES.items():
                if not shutil.which(cfg["tool"]):
                    rows.append({"workload":workload,"language":language,"status":"SKIP","reason":"toolchain unavailable"}); continue
                source=temp/("Main.java" if language=="java" else f"{workload}.{cfg['ext']}"); source.write_text(spec["body"][language]); exe=temp/f"{workload}-{language}"
                build_samples=[]; build_ok=True
                for _ in range(args.repetitions):
                    for command in cfg["build"](source,exe,temp):
                        built, elapsed=invoke(command,temp,args.timeout); build_samples.append(elapsed)
                        if built.returncode: build_ok=False; break
                    if not build_ok: break
                runtime=[]
                if build_ok:
                    for _ in range(args.warmups): invoke(cfg["run"](exe if cfg["build"](source,exe,temp) else source,temp),temp,args.timeout)
                    target=exe if cfg["build"](source,exe,temp) else source
                    for _ in range(args.repetitions):
                        run, elapsed=invoke(cfg["run"](target,temp),temp,args.timeout)
                        if run.returncode==0 and run.stdout.strip()==spec["expected"]: runtime.append(elapsed)
                rows.append({"workload":workload,"language":language,"status":"PASS" if len(runtime)==args.repetitions else ("BUILD-FAIL" if not build_ok else "FAIL"),"build":stats(build_samples) if build_samples else {"median_ms":0.0},"runtime":stats(runtime) if runtime else None,"tool":command_version(cfg["tool"])})
    result={"generated_utc":datetime.now(timezone.utc).isoformat(),"repetitions":args.repetitions,"warmups":args.warmups,"system":{"platform":platform.platform(),"cpu_count":os.cpu_count()},"rows":rows}
    (HERE/"latest.json").write_text(json.dumps(result,indent=2)+"\n")
    lines=["# Cross-language comparison","",f"Generated: `{result['generated_utc']}`; median of {args.repetitions} runs after {args.warmups} warmups.","","| Workload | Language | Build median ms | Runtime median ms | Runtime p95 ms | Status |","|---|---|---:|---:|---:|---|"]
    for r in rows:
        build=r.get("build", {"median_ms":r.get("build_ms",0)}).get("median_ms",0); runtime=r.get("runtime") or {}
        lines.append(f"| {r['workload']} | {r['language']} | {build:.3f} | {runtime.get('median_ms','')} | {runtime.get('p95_ms','')} | {r['status']} |")
    (HERE/"latest.md").write_text("\n".join(lines)+"\n")
    print((HERE/"latest.md").read_text())
if __name__ == "__main__": main()
