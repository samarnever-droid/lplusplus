import time
import subprocess
import os

LPP_FILE = "bench_pillars.lpp"
RUST_FILE = "bench_pillars.rs"
# 10 pillars/tests per set. Roughly 65 lines per set. We want ~100k lines.
NUM_SETS = 1500  # 1500 * 65 = ~97,500 lines

def generate_lpp():
    print(f"Generating {LPP_FILE} with all established pillars...")
    with open(LPP_FILE, "w") as f:
        for i in range(NUM_SETS):
            f.write(f"struct BasicNode{i}:\n")
            f.write(f"    value: Int\n\n")
            
            # Pillar: Stack Value
            f.write(f"def test_value{i}() -> Int:\n")
            f.write(f"    n := BasicNode{i}()\n")
            f.write(f"    return n.value\n\n")
            
            # Pillar: Rule 1 (Return by Reference)
            f.write(f"def test_rule1_{i}() -> BasicNode{i}:\n")
            f.write(f"    n := BasicNode{i}()\n")
            f.write(f"    return n\n\n")
            
            # Pillar: Rule 2 (Closure Capture)
            f.write(f"def test_rule2_{i}() -> Void:\n")
            f.write(f"    x := 100\n")
            f.write(f"    f := fn(y: Int) -> Int:\n")
            f.write(f"        return x + y\n\n")
            
            # Pillar: Rule 4 (Concurrency)
            f.write(f"def test_rule4_{i}() -> Void:\n")
            f.write(f"    shared := BasicNode{i}()\n")
            f.write(f"    spawn fn() -> Void:\n")
            f.write(f"        print(shared.value)\n\n")
            
            # Pillar: Rule 5 (Arenas / Self-Referential)
            f.write(f"struct RecNode{i}:\n")
            f.write(f"    value: Int\n")
            f.write(f"    next: RecNode{i}\n\n")
            
            f.write(f"def test_rule5_{i}() -> Void:\n")
            f.write(f"    r := RecNode{i}()\n\n")

            # 6: If/Else Control Flow
            f.write(f"def test_ifelse_{i}() -> Void:\n")
            f.write(f"    if 1 == 1:\n")
            f.write(f"        print(1)\n")
            f.write(f"    else:\n")
            f.write(f"        print(0)\n\n")

            # 7: While loop
            f.write(f"def test_while_{i}() -> Void:\n")
            f.write(f"    mut count := 0\n")
            f.write(f"    while count < 10:\n")
            f.write(f"        count = count + 1\n\n")

            # 8: Nested Closures
            f.write(f"def test_nested_closures_{i}() -> Void:\n")
            f.write(f"    x := 10\n")
            f.write(f"    f := fn() -> Void:\n")
            f.write(f"        y := 20\n")
            f.write(f"        g := fn() -> Void:\n")
            f.write(f"            print(x + y)\n\n")

            # 9: Rule 3 (Unbounded Container)
            f.write(f"def test_rule3_{i}() -> Void:\n")
            f.write(f"    n := BasicNode{i}()\n")
            f.write(f"    lst := [n]\n\n")

            # 10: Relational Ops
            f.write(f"def test_relational_{i}() -> Void:\n")
            f.write(f"    a := 5\n")
            f.write(f"    b := 10\n")
            f.write(f"    c := a < b\n\n")

def generate_rust():
    print(f"Generating {RUST_FILE} with explicit ARC/Box boilerplate...")
    with open(RUST_FILE, "w") as f:
        f.write("#![allow(warnings)]\n")
        f.write("use std::sync::Arc;\n")
        f.write("use std::thread;\n\n")
        
        for i in range(NUM_SETS):
            f.write(f"struct BasicNode{i} {{\n")
            f.write(f"    value: i32,\n")
            f.write(f"}}\n\n")
            
            # Pillar: Stack Value
            f.write(f"fn test_value{i}() -> i32 {{\n")
            f.write(f"    let n = BasicNode{i} {{ value: 0 }};\n")
            f.write(f"    n.value\n")
            f.write(f"}}\n\n")
            
            # Pillar: Rule 1 (Return by Reference -> Requires Arc or Box)
            f.write(f"fn test_rule1_{i}() -> Arc<BasicNode{i}> {{\n")
            f.write(f"    let n = Arc::new(BasicNode{i} {{ value: 0 }});\n")
            f.write(f"    n\n")
            f.write(f"}}\n\n")
            
            # Pillar: Rule 2 (Closure Capture)
            f.write(f"fn test_rule2_{i}() {{\n")
            f.write(f"    let x = Arc::new(100);\n")
            f.write(f"    let x_clone = x.clone();\n")
            f.write(f"    let f = move |y: i32| -> i32 {{\n")
            f.write(f"        *x_clone + y\n")
            f.write(f"    }};\n")
            f.write(f"}}\n\n")
            
            # Pillar: Rule 4 (Concurrency)
            f.write(f"fn test_rule4_{i}() {{\n")
            f.write(f"    let shared = Arc::new(BasicNode{i} {{ value: 0 }});\n")
            f.write(f"    let shared_clone = shared.clone();\n")
            f.write(f"    thread::spawn(move || {{\n")
            f.write(f"        println!(\"{{}}\", shared_clone.value);\n")
            f.write(f"    }});\n")
            f.write(f"}}\n\n")
            
            # Pillar: Rule 5 (Arenas / Self-Referential)
            f.write(f"struct RecNode{i} {{\n")
            f.write(f"    value: i32,\n")
            f.write(f"    next: Option<Box<RecNode{i}>>,\n")
            f.write(f"}}\n\n")
            
            f.write(f"fn test_rule5_{i}() {{\n")
            f.write(f"    let r = RecNode{i} {{ value: 0, next: None }};\n")
            f.write(f"}}\n\n")

            # 6: If/Else Control Flow
            f.write(f"fn test_ifelse_{i}() {{\n")
            f.write(f"    if 1 == 1 {{\n")
            f.write(f"        println!(\"1\");\n")
            f.write(f"    }} else {{\n")
            f.write(f"        println!(\"0\");\n")
            f.write(f"    }}\n")
            f.write(f"}}\n\n")

            # 7: While loop
            f.write(f"fn test_while_{i}() {{\n")
            f.write(f"    let mut count = 0;\n")
            f.write(f"    while count < 10 {{\n")
            f.write(f"        count += 1;\n")
            f.write(f"    }}\n")
            f.write(f"}}\n\n")

            # 8: Nested Closures
            f.write(f"fn test_nested_closures_{i}() {{\n")
            f.write(f"    let x = Arc::new(10);\n")
            f.write(f"    let x_clone1 = x.clone();\n")
            f.write(f"    let f = move || {{\n")
            f.write(f"        let y = Arc::new(20);\n")
            f.write(f"        let y_clone = y.clone();\n")
            f.write(f"        let x_clone2 = x_clone.clone();\n")
            f.write(f"        let g = move || {{\n")
            f.write(f"            println!(\"{{}}\", *x_clone2 + *y_clone);\n")
            f.write(f"        }};\n")
            f.write(f"    }};\n")
            f.write(f"}}\n\n")

            # 9: Rule 3 (Unbounded Container)
            f.write(f"fn test_rule3_{i}() {{\n")
            f.write(f"    let n = Arc::new(BasicNode{i} {{ value: 0 }});\n")
            f.write(f"    let lst = vec![n.clone()];\n")
            f.write(f"}}\n\n")

            # 10: Relational Ops
            f.write(f"fn test_relational_{i}() {{\n")
            f.write(f"    let a = 5;\n")
            f.write(f"    let b = 10;\n")
            f.write(f"    let c = a < b;\n")
            f.write(f"}}\n\n")
            
        f.write("fn main() {}\n")

def run_benchmark():
    subprocess.run(["cargo", "build", "--release"], check=True)
    lpp_executable = os.path.join("target", "release", "lpp.exe")

    print("Benchmarking L++ (Frontend Analysis + Escape Rules)...")
    start = time.time()
    subprocess.run([lpp_executable, LPP_FILE], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    lpp_time = time.time() - start

    print("Benchmarking Rust (rustc parsing, macros, and full compile)...")
    start = time.time()
    subprocess.run(["rustc", RUST_FILE], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    rust_time = time.time() - start

    print("-" * 30)
    print("ALL PILLARS BENCHMARK RESULTS (~100,000 lines)")
    print("-" * 30)
    print(f"L++ Escape Analysis Time: {lpp_time:.4f} seconds")
    print(f"Rustc Compile Time:       {rust_time:.4f} seconds")
    
    if lpp_time < rust_time:
        speedup = rust_time / lpp_time
        print(f"L++ frontend is {speedup:.2f}x faster!")
    else:
        speedup = lpp_time / rust_time
        print(f"Rust is {speedup:.2f}x faster!")

if __name__ == "__main__":
    generate_lpp()
    generate_rust()
    run_benchmark()
