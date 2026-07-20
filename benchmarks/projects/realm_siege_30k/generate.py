#!/usr/bin/env python3
"""Generate six equivalent 30k-line Realm Siege game implementations."""
from pathlib import Path

OUT = Path(__file__).parent / "generated"
ROOMS = 5000

def lpp():
    a=['# Realm Siege 30K — generated L++ tower-defense simulation.','',
       'def clamp(v: Int, low: Int, high: Int) -> Int:','    if v < low:','        return low','    else:','        if v > high:','            return high','        else:','            return v','',
       'def main():','    mut gold := 250','    mut score := 0','    mut wave := 0','    while wave < 120:','        gold = gold + wave % 7','        score = score + realm_room_0000(wave)','        wave = wave + 1','    print(score + gold)']
    for i in range(ROOMS):
        n=i%97+1; a += ['',f'def realm_room_{i:04d}(wave: Int) -> Int:',f'    enemy := wave * {n} + {i%31}',f'    tower := {i%13+3} * (wave % 11)', '    if enemy % 3 == 0:', '        return clamp(enemy - tower, 0, 1000000)', '    else:', '        return clamp(enemy + tower + 1, 0, 1000000)']
    return '\n'.join(a)+'\n'

def c_like(lang):
    if lang=='c': head='#include <stdio.h>\nstatic long long clamp(long long v,long long l,long long h){return v<l?l:(v>h?h:v);}\n'; fn=lambda i,n:f'static long long room_{i}(long long w){{\n long long e=w*{n}+{i%31};\n long long t={i%13+3}*(w%11);\n if(e%3==0) return clamp(e-t,0,1000000);\n return clamp(e+t+1,0,1000000);\n}}\n'; main='int main(){long long g=250,s=0;for(long long w=0;w<120;w++){g+=w%7;s+=room_0(w);}printf("%lld\\n",s+g);}\n'
    elif lang=='cpp': head='#include <cstdio>\nstatic long long clamp(long long v,long long l,long long h){return v<l?l:(v>h?h:v);}\n'; fn=lambda i,n:f'static long long room_{i}(long long w){{\n long long e=w*{n}+{i%31};\n long long t={i%13+3}*(w%11);\n if(e%3==0) return clamp(e-t,0,1000000);\n return clamp(e+t+1,0,1000000);\n}}\n'; main='int main(){long long g=250,s=0;for(long long w=0;w<120;w++){g+=w%7;s+=room_0(w);}std::printf("%lld\\n",s+g);}\n'
    elif lang=='rust': head='fn clamp(v:i64,l:i64,h:i64)->i64{if v<l{l}else if v>h{h}else{v}}\n'; fn=lambda i,n:f'fn room_{i}(w:i64)->i64{{\n let e=w*{n}+{i%31};\n let t={i%13+3}*(w%11);\n if e%3==0{{\n  clamp(e-t,0,1000000)\n }}else{{\n  clamp(e+t+1,0,1000000)\n }}\n}}\n'; main='fn main(){let mut g=250i64;let mut s=0i64;for w in 0..120i64{g+=w%7;s+=room_0(w);}println!("{}",s+g);}\n'
    elif lang=='go': head='package main\nimport "fmt"\nfunc clamp(v,l,h int64)int64{if v<l{return l};if v>h{return h};return v}\n'; fn=lambda i,n:f'func room_{i}(w int64)int64{{\n e:=w*{n}+{i%31}\n t:=int64({i%13+3})*(w%11)\n if e%3==0{{\n  return clamp(e-t,0,1000000)\n }}\n return clamp(e+t+1,0,1000000)\n}}\n'; main='func main(){var g int64=250;var s int64;for w:=int64(0);w<120;w++{g+=w%7;s+=room_0(w)};fmt.Println(s+g)}\n'
    else: # java
        head='public class RealmSiege{static long clamp(long v,long l,long h){return v<l?l:(v>h?h:v);}\n'; fn=lambda i,n:f'static long room_{i}(long w){{\n long e=w*{n}+{i%31};\n long t={i%13+3}*(w%11);\n if(e%3==0) return clamp(e-t,0,1000000);\n return clamp(e+t+1,0,1000000);\n}}\n'; main='public static void main(String[]x){long g=250,s=0;for(long w=0;w<120;w++){g+=w%7;s+=room_0(w);}System.out.println(s+g);}}\n'
    return head+''.join(fn(i,i%97+1) for i in range(ROOMS))+main

OUT.mkdir(parents=True,exist_ok=True)
files={'realm_siege.lpp':lpp(),'realm_siege.c':c_like('c'),'realm_siege.cpp':c_like('cpp'),'realm_siege.rs':c_like('rust'),'realm_siege.go':c_like('go'),'RealmSiege.java':c_like('java')}
for name,body in files.items():
    path=OUT/name; path.write_text(body); print(f'{path}: {body.count(chr(10))} lines')
    if body.count('\n') < 30000: raise SystemExit(f'{name} below 30k lines')

# Multi-file L++ package layout. The same generated game is split into gameplay
# modules so `lpp build` exercises import resolution and package compilation.
def multi_file_lpp_project():
    root = OUT / "lpp_project"
    src = root / "src"
    src.mkdir(parents=True, exist_ok=True)
    full = lpp().splitlines()
    first_room = next(i for i, line in enumerate(full) if line.startswith("def realm_room_"))
    header = full[:first_room]
    # Imports precede declarations; the resolver merges local modules into the
    # project AST while retaining the normal single-package entry point.
    imports = [f"import rooms_{i}" for i in range(5)] + [""]
    (src / "main.lpp").write_text("\n".join(imports + header) + "\n")
    rooms = full[first_room:]
    # Each room definition is eight source lines plus one separator line.
    chunk = ROOMS // 5
    for module in range(5):
        begin, end = module * chunk * 8, (module + 1) * chunk * 8
        (src / f"rooms_{module}.lpp").write_text("\n".join(rooms[begin:end]) + "\n")
    (root / "lpp.toml").write_text('''[package]
name = "realm_siege_30k"
version = "0.1.0"
entry = "src/main.lpp"
''')
    total = sum(1 for path in src.glob("*.lpp") for _ in path.open())
    print(f"{root}: {total} L++ lines across {len(list(src.glob('*.lpp')))} source files")

multi_file_lpp_project()
