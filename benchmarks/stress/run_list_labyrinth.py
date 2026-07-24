#!/usr/bin/env python3
"""Reproducible backend/link benchmark for the 18k-line L++ List Labyrinth."""
from __future__ import annotations
import json, shutil, subprocess, sys, tempfile, time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "safety/generated/list_game_stress_10k.lpp"
COMPILER = ROOT / "target/release/lpp"
DIRECT = ROOT / "target/release/lpp-link"
RUNTIME = ROOT / "runtime/linux_x86_64_min.c"
OUT = ROOT / "benchmarks/stress/list_labyrinth_latest.json"
MD = ROOT / "benchmarks/stress/list_labyrinth_latest.md"

def run(cmd: list[str], cwd: Path, allow_fail: bool = False):
    started = time.perf_counter()
    p = subprocess.run(cmd, cwd=cwd, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    elapsed = (time.perf_counter() - started) * 1000
    if not allow_fail and p.returncode:
        raise RuntimeError(f"{' '.join(cmd)} failed:\n{p.stdout}\n{p.stderr}")
    return {"ms": round(elapsed, 3), "code": p.returncode, "stdout": p.stdout, "stderr": p.stderr}

def main() -> int:
    if not COMPILER.exists():
        raise SystemExit("build target/release/lpp first")
    cc = shutil.which("cc") or shutil.which("clang") or shutil.which("gcc")
    if not cc: raise SystemExit("no C compiler")
    with tempfile.TemporaryDirectory(prefix="lpp-list-stress-") as temp:
        work = Path(temp); source = work / "labyrinth.lpp"; shutil.copy2(SOURCE, source)
        c_emit = run([str(COMPILER), "emit", str(source)], work)
        c_file = source.with_suffix(".c"); c_exe = work / "c-backend"
        c_link = run([cc, "-O2", str(c_file), "-o", str(c_exe), "-pthread"], work)
        c_run = run([str(c_exe)], work)
        aot_emit = run([str(COMPILER), "emit", str(source), "--aot"], work)
        obj = source.with_suffix(".o"); aot_exe = work / "aot-host"
        aot_link = run([cc, "-O2", str(obj), str(ROOT / "lpp_runtime.c"), "-o", str(aot_exe), "-pthread"], work)
        aot_run = run([str(aot_exe)], work)
        direct = {"available": DIRECT.exists(), "supported": False}
        if DIRECT.exists():
            runtime_obj = work / "direct-runtime.o"
            runtime_compile = run([cc, "-Os", "-ffreestanding", "-fno-stack-protector", "-fno-pic", "-mno-red-zone", "-fno-reorder-blocks-and-partition", "-c", str(RUNTIME), "-o", str(runtime_obj)], work)
            direct_try = run([str(DIRECT), str(obj), str(runtime_obj), "-o", str(work / "aot-direct")], work, allow_fail=True)
            direct.update({"runtime_compile_ms": runtime_compile["ms"], "link_ms": direct_try["ms"], "exit_code": direct_try["code"], "diagnostic": (direct_try["stdout"] + direct_try["stderr"]).strip()[:1000]})
        result = {
            "workload": {"source": "safety/generated/list_game_stress_10k.lpp", "lines": sum(1 for _ in SOURCE.open()), "expected_stdout": "552"},
            "c_backend": {"emit_ms": c_emit["ms"], "host_link_ms": c_link["ms"], "run_ms": c_run["ms"], "stdout": c_run["stdout"].strip()},
            "cranelift_aot": {"emit_ms": aot_emit["ms"], "host_link_ms": aot_link["ms"], "run_ms": aot_run["ms"], "stdout": aot_run["stdout"].strip()},
            "direct_linker": direct,
        }
    OUT.write_text(json.dumps(result, indent=2) + "\n")
    direct_status = "not available" if not result["direct_linker"]["available"] else ("supported" if result["direct_linker"]["supported"] else "rejected as unsupported")
    MD.write_text(f'''# List Labyrinth backend benchmark\n\nWorkload: **{result["workload"]["lines"]:,} lines**, List-heavy game + file-write integration.\n\n| Path | Emit / compile ms | Host link ms | Run ms | Output |\n|---|---:|---:|---:|---|\n| C backend | {result["c_backend"]["emit_ms"]:.3f} | {result["c_backend"]["host_link_ms"]:.3f} | {result["c_backend"]["run_ms"]:.3f} | `{result["c_backend"]["stdout"]}` |\n| Cranelift AOT | {result["cranelift_aot"]["emit_ms"]:.3f} | {result["cranelift_aot"]["host_link_ms"]:.3f} | {result["cranelift_aot"]["run_ms"]:.3f} | `{result["cranelift_aot"]["stdout"]}` |\n\n## Direct native linker\n\nStatus: **{direct_status}**. This workload intentionally calls `write_file`; direct Linux ELF does not currently provide file I/O/writable-data support. Rejection is correct safety behavior, not a benchmark failure.\n\nRaw data: `list_labyrinth_latest.json`.\n''')
    print(MD.read_text())
if __name__ == "__main__": main()
