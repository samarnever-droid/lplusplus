#!/usr/bin/env python3
"""Single-run canonical L++/C/C++/Rust/Go/Zig comparison harness."""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent

WORKLOADS = {
    "fib35": {
        "expected": "9227465",
        "lpp": ROOT / "benchmarks/bench_fib.lpp",
        "body": {
            "c": "#include <stdio.h>\nlong long fib(long long n){return n<=1?n:fib(n-1)+fib(n-2);}\nint main(void){printf(\"%lld\\n\",fib(35));return 0;}\n",
            "cpp": "#include <cstdio>\nlong long fib(long long n){return n<=1?n:fib(n-1)+fib(n-2);}\nint main(){std::printf(\"%lld\\n\",fib(35));}\n",
            "rust": "fn fib(n:i64)->i64{if n<=1{n}else{fib(n-1)+fib(n-2)}} fn main(){println!(\"{}\",fib(35));}\n",
            "go": "package main\nimport \"fmt\"\nfunc fib(n int64) int64 { if n<=1{return n}; return fib(n-1)+fib(n-2) }\nfunc main(){fmt.Println(fib(35))}\n",
            "zig": "const std=@import(\"std\"); fn fib(n:i64)i64{return if(n<=1)n else fib(n-1)+fib(n-2);} pub fn main()!void{std.debug.print(\"{}\\n\",.{fib(35)});}\n",
        },
    },
    "loop10m": {
        "expected": "49999995000000",
        "lpp": ROOT / "benchmarks/bench_loop.lpp",
        "body": {
            "c": "#include <stdio.h>\nint main(void){long long a=0;for(long long i=0;i<10000000;i++)a+=i;printf(\"%lld\\n\",a);return 0;}\n",
            "cpp": "#include <cstdio>\nint main(){long long a=0;for(long long i=0;i<10000000;i++)a+=i;std::printf(\"%lld\\n\",a);}\n",
            "rust": "fn main(){let mut a:i64=0;for i in 0..10_000_000i64{a+=i;}println!(\"{}\",a);}\n",
            "go": "package main\nimport \"fmt\"\nfunc main(){var a int64;for i:=int64(0);i<10000000;i++{a+=i};fmt.Println(a)}\n",
            "zig": "const std=@import(\"std\"); pub fn main()!void{var a:i64=0;var i:i64=0;while(i<10000000):(i+=1){a+=i;}std.debug.print(\"{}\\n\",.{a});}\n",
        },
    },
}

LANGUAGES = {
    "c": {"tool": "cc", "extension": "c", "build": lambda src, exe: ["cc", "-O3", str(src), "-o", str(exe)]},
    "cpp": {"tool": "c++", "extension": "cpp", "build": lambda src, exe: ["c++", "-O3", str(src), "-o", str(exe)]},
    "rust": {"tool": "rustc", "extension": "rs", "build": lambda src, exe: ["rustc", "-O", str(src), "-o", str(exe)]},
    "go": {"tool": "go", "extension": "go", "build": lambda src, exe: ["go", "build", "-o", str(exe), str(src)]},
    "zig": {"tool": "zig", "extension": "zig", "build": lambda src, exe: ["zig", "build-exe", "-O", "ReleaseFast", str(src), "-femit-bin=" + str(exe)]},
}


def version(tool: str) -> str:
    try:
        return subprocess.check_output([tool, "--version"], text=True, stderr=subprocess.STDOUT).splitlines()[0]
    except Exception:
        return "unavailable"


def run_executable(exe: Path) -> tuple[float, subprocess.CompletedProcess[str]]:
    started = time.perf_counter()
    result = subprocess.run([str(exe)], text=True, capture_output=True)
    return (time.perf_counter() - started) * 1000, result


def main() -> None:
    if not shutil.which("cargo") or not shutil.which("cc"):
        raise SystemExit("L++ comparison requires cargo and cc")
    subprocess.run(["cargo", "build", "--release"], cwd=ROOT, check=True)
    lpp = ROOT / "target/release" / ("lpp.exe" if os.name == "nt" else "lpp")
    rows = []
    with tempfile.TemporaryDirectory(prefix="lpp-compare-") as td:
        temp = Path(td)
        for workload, spec in WORKLOADS.items():
            # L++ AOT baseline
            lpp_src = temp / f"{workload}.lpp"
            lpp_src.write_text(spec["lpp"].read_text())
            env = os.environ.copy(); env.update({"LPP_AOT": "1", "LPP_RELEASE": "1"})
            compile_started = time.perf_counter()
            subprocess.run([str(lpp), str(lpp_src)], env=env, check=True, capture_output=True)
            compile_ms = (time.perf_counter() - compile_started) * 1000
            lpp_exe = temp / f"{workload}-lpp"
            link_started = time.perf_counter()
            subprocess.run(["cc", "-O2", str(lpp_src.with_suffix('.o')), str(ROOT/'lpp_runtime.c'), "-o", str(lpp_exe), "-pthread"], check=True)
            link_ms = (time.perf_counter() - link_started) * 1000
            runtime, execution = run_executable(lpp_exe)
            rows.append({
                "workload": workload, "language": "lpp",
                "status": "PASS" if execution.returncode == 0 and execution.stdout.strip() == spec["expected"] else "FAIL",
                "compile_ms": compile_ms, "link_ms": link_ms,
                "build_ms": compile_ms + link_ms, "runtime_ms": runtime,
                "tool": "Cranelift AOT + host link",
            })
            for language, config in LANGUAGES.items():
                if not shutil.which(config["tool"]):
                    rows.append({"workload": workload, "language": language, "status": "SKIP", "runtime_ms": None, "tool": "unavailable"})
                    continue
                src = temp / f"{workload}.{config['extension']}"
                exe = temp / f"{workload}-{language}"
                src.write_text(spec["body"][language])
                started = time.perf_counter()
                build = subprocess.run(config["build"](src, exe), text=True, capture_output=True)
                build_ms = (time.perf_counter() - started) * 1000
                if build.returncode != 0:
                    rows.append({"workload": workload, "language": language, "status": "BUILD-FAIL", "runtime_ms": None, "compile_ms": build_ms, "link_ms": None, "build_ms": build_ms, "tool": version(config["tool"])})
                    continue
                runtime, execution = run_executable(exe)
                rows.append({"workload": workload, "language": language, "status": "PASS" if execution.returncode == 0 and execution.stdout.strip() == spec["expected"] else "FAIL", "runtime_ms": runtime, "compile_ms": build_ms, "link_ms": None, "build_ms": build_ms, "tool": version(config["tool"])})
    result = {"generated_utc": datetime.now(timezone.utc).isoformat(), "rows": rows}
    (HERE / "latest.json").write_text(json.dumps(result, indent=2) + "\n")
    lines = [
        "# Cross-language comparison", "", f"Generated: `{result['generated_utc']}`", "",
        "| Workload | Language | Compile ms | Link ms | Total build ms | Runtime ms | Status |",
        "|---|---|---:|---:|---:|---:|---|",
    ]
    for row in rows:
        compile_ms = "" if row.get("compile_ms") is None else f"{row['compile_ms']:.3f}"
        link_ms = "" if row.get("link_ms") is None else f"{row['link_ms']:.3f}"
        build_ms = "" if row.get("build_ms") is None else f"{row['build_ms']:.3f}"
        runtime = "" if row.get("runtime_ms") is None else f"{row['runtime_ms']:.3f}"
        lines.append(f"| {row['workload']} | {row['language']} | {compile_ms} | {link_ms} | {build_ms} | {runtime} | {row['status']} |")
    (HERE / "latest.md").write_text("\n".join(lines) + "\n")
    print((HERE / "latest.md").read_text())


if __name__ == "__main__":
    main()
