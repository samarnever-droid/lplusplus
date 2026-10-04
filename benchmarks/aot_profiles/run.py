#!/usr/bin/env python3
"""Compare Cranelift AOT optimisation profiles on a fixed 100k LOC source."""
from __future__ import annotations
import json, os, shutil, subprocess, sys, tempfile, time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
SOURCE=ROOT/'benchmarks/scalability/generated/scale_100000.lpp'
COMPILER=ROOT/'target/release/lpp'
PROFILES=('none','speed_and_size','speed')
def run(cmd, env, cwd):
 t=time.perf_counter(); p=subprocess.run(cmd,cwd=cwd,env=env,text=True,capture_output=True)
 return p,(time.perf_counter()-t)*1000
def main():
 if not SOURCE.exists(): subprocess.run([sys.executable,str(ROOT/'benchmarks/scalability/generate.py')],check=True)
 cc=shutil.which('cc') or 'cc'; rows=[]
 with tempfile.TemporaryDirectory(prefix='lpp-aot-profiles-') as d:
  d=Path(d)
  for profile in PROFILES:
   src=d/f'{profile}.lpp'; shutil.copy2(SOURCE,src); env=os.environ|{'LPP_AOT':'1','LPP_AOT_OPT':profile,'BENCHMARK':'1'}
   p,emit=run([str(COMPILER),str(src)],env,d)
   obj=src.with_suffix('.o')
   if p.returncode or not obj.exists(): raise RuntimeError(p.stderr)
   exe=d/profile; p,link=run([cc,'-O2',str(obj),str(ROOT/'lpp_runtime.c'),'-o',str(exe),'-pthread'],os.environ,d)
   if p.returncode: raise RuntimeError(p.stderr)
   p,run_ms=run([str(exe)],os.environ,d)
   if p.returncode or p.stdout.strip()!='99996': raise RuntimeError(p.stderr+p.stdout)
   rows.append({'profile':profile,'aot_emit_ms':round(emit,3),'host_link_ms':round(link,3),'run_ms':round(run_ms,3),'object_bytes':obj.stat().st_size})
 out={'source_lines':100000,'rows':rows}; (ROOT/'benchmarks/aot_profiles/latest.json').write_text(json.dumps(out,indent=2)+'\n')
 md=['# Cranelift AOT optimisation profiles','', '| Profile | AOT emit ms | Host link ms | Run ms | Object bytes |','|---|---:|---:|---:|---:|']
 md += [f"| {r['profile']} | {r['aot_emit_ms']:.3f} | {r['host_link_ms']:.3f} | {r['run_ms']:.3f} | {r['object_bytes']} |" for r in rows]
 (ROOT/'benchmarks/aot_profiles/latest.md').write_text('\n'.join(md)+'\n'); print('\n'.join(md))
if __name__=='__main__': main()
