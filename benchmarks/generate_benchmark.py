import time
import subprocess
import os

LPP_FILE = "bench.lpp"
RUST_FILE = "bench.rs"
NUM_FUNCTIONS = 20000  # 20k functions, approx 100k lines

def generate_lpp():
    print(f"Generating {LPP_FILE}...")
    with open(LPP_FILE, "w") as f:
        for i in range(NUM_FUNCTIONS):
            f.write(f"struct Node{i}:\n")
            f.write(f"    value: Int\n\n")
            f.write(f"def func{i}() -> Node{i}:\n")
            f.write(f"    n := Node{i}()\n")
            f.write(f"    return n\n\n")
            
def generate_rust():
    print(f"Generating {RUST_FILE}...")
    with open(RUST_FILE, "w") as f:
        for i in range(NUM_FUNCTIONS):
            f.write(f"struct Node{i} {{\n")
            f.write(f"    value: i32,\n")
            f.write(f"}}\n\n")
            f.write(f"fn func{i}() -> Node{i} {{\n")
            f.write(f"    let n = Node{i} {{ value: 0 }};\n")
            f.write(f"    n\n")
            f.write(f"}}\n\n")
        f.write("fn main() {}\n")

def run_benchmark():
    # Build L++ compiler in release mode first
    print("Building L++ compiler in release mode...")
    subprocess.run(["cargo", "build", "--release"], check=True)
    lpp_executable = os.path.join("target", "release", "lpp.exe")

    # Benchmark L++
    print("Benchmarking L++ (Frontend Analysis)...")
    start = time.time()
    # Suppress output so it doesn't flood the terminal
    subprocess.run([lpp_executable, LPP_FILE], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    lpp_time = time.time() - start
    print(f"L++ Time: {lpp_time:.4f} seconds")

    # Benchmark Rust
    print("Benchmarking Rust (rustc)...")
    start = time.time()
    subprocess.run(["rustc", RUST_FILE], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    rust_time = time.time() - start
    print(f"Rust Time: {rust_time:.4f} seconds")

    print("-" * 30)
    print("BENCHMARK RESULTS (approx 100,000 lines of code)")
    print("-" * 30)
    print(f"L++ (Frontend):   {lpp_time:.4f} seconds")
    print(f"Rust (rustc):     {rust_time:.4f} seconds")
    
    if lpp_time < rust_time:
        speedup = rust_time / lpp_time
        print(f"L++ is {speedup:.2f}x faster!")
    else:
        speedup = lpp_time / rust_time
        print(f"Rust is {speedup:.2f}x faster!")
        
    print("\n*Note: L++ currently only runs lexing, parsing, and semantic/escape analysis.")
    print("Rust (rustc) is doing full compilation to native machine code.")

if __name__ == "__main__":
    generate_lpp()
    generate_rust()
    run_benchmark()
