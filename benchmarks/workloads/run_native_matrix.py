#!/usr/bin/env python3
"""Run L++ workload-shape matrix through C and Cranelift AOT native paths."""
from __future__ import annotations
import json, shutil, subprocess, tempfile, time
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
WORKLOADS = {
    "arithmetic": (ROOT / "benchmarks/bench_loop.lpp", "49999995000000"),
    "branches": (ROOT / "benchmarks/bench_branch.lpp", "6666669"),
    "calls": (ROOT / "benchmarks/bench_function_heavy.lpp", "149999995000000"),
    "struct_list": (ROOT / "benchmarks/bench_struct_list.lpp", "400005"),
    "list_labyrinth": (ROOT / "safety/generated/list_game_stress_10k.lpp", "552"),
}

def command(cmd, cwd, env=None):
    started=time.perf_counter(); p=subprocess.run(cmd,cwd=cwd,env=env,text=True,capture_output=True)
    return p,(time.perf_counter()-started)*1000

def main():
    compiler=ROOT/"target/release/lpp"; cc=shutil.which("cc")
    if not compiler.exists() or not cc: raise SystemExit("build target/release/lpp and install cc")
    rows=[]
    with tempfile.TemporaryDirectory(prefix="lpp-native-matrix-") as raw:
        work=Path(raw)
        for name,(source,expected) in WORKLOADS.items():
            if not source.exists(): raise SystemExit(f"missing {source}")
            for backend in ("c", "aot"):
                src=work/f"{name}-{backend}.lpp"; shutil.copy2(source,src)
                cmd=[str(compiler),"emit",str(src)] + (["--aot"] if backend=="aot" else [])
                p,emit=command(cmd,work)
                if p.returncode: raise RuntimeError(p.stderr)
                exe=work/f"{name}-{backend}"
                inputs=[str(src.with_suffix(".o")),str(ROOT/"lpp_runtime.c")] if backend=="aot" else [str(src.with_suffix(".c"))]
                p,link=command([cc,"-O2",*inputs,"-o",str(exe),"-pthread"],work)
                if p.returncode: raise RuntimeError(p.stderr)
                p,run=command([str(exe)],work)
                rows.append({"workload":name,"backend":backend,"status":"PASS" if p.returncode==0 and p.stdout.strip()==expected else "FAIL","emit_ms":round(emit,3),"link_ms":round(link,3),"run_ms":round(run,3),"stdout":p.stdout.strip()})
    result={"generated_utc":datetime.now(timezone.utc).isoformat(),"rows":rows}
    (HERE/"latest.json").write_text(json.dumps(result,indent=2)+"\n")
    lines=["# Native workload-shape matrix","","| Workload | Backend | Emit ms | Link ms | Run ms | Status |","|---|---|---:|---:|---:|---|"]
    lines += [f"| {r['workload']} | {r['backend']} | {r['emit_ms']:.3f} | {r['link_ms']:.3f} | {r['run_ms']:.3f} | {r['status']} |" for r in rows]
    (HERE/"latest.md").write_text("\n".join(lines)+"\n")
    print((HERE/"latest.md").read_text())
if __name__=="__main__": main()
