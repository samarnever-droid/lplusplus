#!/usr/bin/env python3
import json
import os
import subprocess
import time
import tempfile
from pathlib import Path

ROOT = Path("/home/user/repo")
manifest = json.loads((ROOT / "benchmarks/king20/experimental/manifest.json").read_text())
compiler = ROOT / "target/release/lpp"
direct_linker = ROOT / "target/release/lpp-link"
cc = os.environ.get("CC", "cc")

runtime_obj = Path("/home/user/.lpp/lib/lpp_runtime.o")
if not runtime_obj.exists():
    runtime_obj = ROOT / "LppData/lib/lpp_runtime.o"

runtime_min_obj = Path("/home/user/.lpp/lib/lpp_runtime_min.o")
if not runtime_min_obj.exists():
    runtime_min_obj = ROOT / "LppData/lib/lpp_runtime_min.o"

results = []

with tempfile.TemporaryDirectory(prefix="lpp-3linkers-") as temp_dir:
    temp = Path(temp_dir)
    for case in manifest["benchmarks"]:
        source = ROOT / case["source"]
        case_id = case["id"]
        case_name = case["name"]
        copied = temp / f"{case_id:02d}_{source.name}"
        copied.write_text(source.read_text())
        
        # Compile to AOT object
        env = os.environ.copy()
        env.update({"LPP_AOT": "1", "BENCHMARK": "1", "LPP_RELEASE": "1"})
        compile_run = subprocess.run([str(compiler), str(copied)], env=env, text=True, capture_output=True)
        timing_lines = [line.split("TIMING_JSON: ", 1)[1] for line in compile_run.stdout.splitlines() if line.startswith("TIMING_JSON:")]
        if not timing_lines:
            print(f"Error compiling case {case_id}: {compile_run.stderr}")
            continue
        timing = json.loads(timing_lines[0])
        obj = copied.with_suffix(".o")
        
        # 1. Host Linker (recompiling lpp_runtime.c)
        exe_host_c = temp / f"host_c_{case_id}"
        t0 = time.perf_counter()
        subprocess.run([cc, "-O2", str(obj), str(ROOT / "lpp_runtime.c"), "-o", str(exe_host_c), "-pthread"], check=True, capture_output=True)
        link_host_recompile_c_ms = (time.perf_counter() - t0) * 1000.0

        # 2. Host Linker (prebuilt lpp_runtime.o)
        exe_host_obj = temp / f"host_obj_{case_id}"
        t0 = time.perf_counter()
        subprocess.run([cc, "-O2", str(obj), str(runtime_obj), "-o", str(exe_host_obj), "-pthread"], check=True, capture_output=True)
        link_host_prebuilt_ms = (time.perf_counter() - t0) * 1000.0
        
        # 3. Mold Linker (-fuse-ld=mold with prebuilt lpp_runtime.o)
        exe_mold = temp / f"mold_{case_id}"
        t0 = time.perf_counter()
        subprocess.run([cc, "-fuse-ld=mold", "-O2", str(obj), str(runtime_obj), "-o", str(exe_mold), "-pthread"], check=True, capture_output=True)
        link_mold_ms = (time.perf_counter() - t0) * 1000.0
        
        # 4. Direct Linker (lpp-link with lpp_runtime_min.o)
        exe_direct = temp / f"direct_{case_id}"
        t0 = time.perf_counter()
        subprocess.run([str(direct_linker), str(obj), str(runtime_min_obj), "-o", str(exe_direct)], check=True, capture_output=True)
        link_direct_ms = (time.perf_counter() - t0) * 1000.0
        
        results.append({
            "id": case_id,
            "name": case_name,
            "compile_ms": round(timing["total"] * 1000, 3),
            "aot_ms": round(timing["aot"] * 1000, 3),
            "link_host_recompile_c_ms": round(link_host_recompile_c_ms, 3),
            "link_host_prebuilt_obj_ms": round(link_host_prebuilt_ms, 3),
            "link_mold_ms": round(link_mold_ms, 3),
            "link_direct_ms": round(link_direct_ms, 3),
            "speedup_mold_vs_recompile": f"{link_host_recompile_c_ms / link_mold_ms:.1f}x" if link_mold_ms > 0 else "N/A",
            "speedup_direct_vs_recompile": f"{link_host_recompile_c_ms / link_direct_ms:.1f}x" if link_direct_ms > 0 else "N/A"
        })

output_data = {
    "suite": "L++ King 20 Linker Strategy Benchmarks",
    "compiler_version": "v0.1.3 (release)",
    "linkers": {
        "host_recompile_c": "cc (Debian 14.2.0-19) standard ld + recompile lpp_runtime.c",
        "host_prebuilt_obj": "cc (Debian 14.2.0-19) standard ld + lpp_runtime.o",
        "mold": "mold 2.37.1 via cc -fuse-ld=mold + lpp_runtime.o",
        "direct": "lpp-link (custom ELF direct linker) + lpp_runtime_min.o"
    },
    "results": results
}

output_path = ROOT / "benchmarks/king20/three_linkers_benchmark.json"
output_path.write_text(json.dumps(output_data, indent=2) + "\n")
print(json.dumps(output_data, indent=2))
