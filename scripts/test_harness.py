#!/usr/bin/env python3
"""
L++ Automated Multi-Suite Test Harness & Differential Validation Engine
-----------------------------------------------------------------------
Systematically validates the L++ compiler across all backend pipelines:
1. Cranelift Native Direct Backend (`lpp file.lpp -o bin.exe`)
2. LLVM Native Direct Backend (`lpp file.lpp --llvm -o bin_llvm.exe`)

Key Architectural Guarantees Verified:
- Microsoft x64 ABI calling conventions and stack alignment
- ARC memory safety (uninitialized branch zeroing, struct moves, string stress)
- Custom struct destructors & managed field lifetimes
- String builtins & CommonMark AST parsing
- 100% Differential execution fidelity (Cranelift stdout == LLVM stdout)
"""

import os
import sys
import time
import argparse
import subprocess
import glob
from pathlib import Path

ROOT_DIR = Path(__file__).resolve().parent.parent
CASES_DIR = ROOT_DIR / "tests" / "harness" / "cases"
PACKAGES_DIR = ROOT_DIR / "packages"
BIN_DIR = ROOT_DIR / "target" / "test_binaries"

# Packages that are interactive servers or require stdin/terminal environments
INTERACTIVE_PACKAGES = {"lreact", "lppsqlite", "db-benchmark", "lpp-bindgen", "lppstore"}

GREEN = "\033[92m"
RED = "\033[91m"
YELLOW = "\033[93m"
CYAN = "\033[96m"
BOLD = "\033[1m"
RESET = "\033[0m"


def run_command(cmd, cwd=None, timeout=15):
    start = time.perf_counter()
    env = dict(os.environ)
    env["LPP_TEST_MODE"] = "1"
    try:
        proc = subprocess.run(
            cmd,
            cwd=cwd,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=timeout,
        )
        duration = time.perf_counter() - start
        return {
            "success": proc.returncode == 0,
            "returncode": proc.returncode,
            "stdout": proc.stdout,
            "stderr": proc.stderr,
            "duration_ms": round(duration * 1000, 2),
        }
    except subprocess.TimeoutExpired:
        return {
            "success": False,
            "returncode": -999,
            "stdout": "",
            "stderr": f"Execution timed out after {timeout}s",
            "duration_ms": timeout * 1000,
        }
    except Exception as e:
        return {
            "success": False,
            "returncode": -998,
            "stdout": "",
            "stderr": str(e),
            "duration_ms": 0,
        }


def main():
    parser = argparse.ArgumentParser(description="L++ Automated Multi-Suite Test Harness")
    parser.add_argument("--suite", choices=["all", "core", "packages"], default="all",
                        help="Which test suite to run (default: all)")
    args = parser.parse_args()

    print(f"\n{BOLD}{CYAN}=================================================================={RESET}")
    print(f"{BOLD}{CYAN}   L++ AUTOMATED MULTI-SUITE TEST HARNESS & DIFFERENTIAL VALIDATION   {RESET}")
    print(f"{BOLD}{CYAN}=================================================================={RESET}\n")

    BIN_DIR.mkdir(parents=True, exist_ok=True)

    all_tests = []

    if args.suite in ("all", "core"):
        case_files = sorted(glob.glob(str(CASES_DIR / "*.lpp")))
        for cf in case_files:
            p = Path(cf)
            all_tests.append({"name": p.stem, "path": p, "type": "core_case"})

    if args.suite in ("all", "packages"):
        package_mains = sorted(glob.glob(str(PACKAGES_DIR / "*" / "src" / "main.lpp")))
        for pm in package_mains:
            p = Path(pm)
            pkg_name = p.parent.parent.name
            if pkg_name in INTERACTIVE_PACKAGES:
                continue
            all_tests.append({"name": f"pkg_{pkg_name}", "path": p, "type": "package"})

    print(f"Executing {BOLD}{len(all_tests)}{RESET} test cases in suite '{BOLD}{args.suite}{RESET}'...\n")

    results = []
    passed_count = 0
    failed_count = 0

    for idx, test in enumerate(all_tests, 1):
        name = test["name"]
        path = test["path"]
        print(f"[{idx}/{len(all_tests)}] Testing: {BOLD}{name:<32}{RESET} ... ", end="", flush=True)

        cl_exe = BIN_DIR / f"{name}_cl.exe"
        llvm_exe = BIN_DIR / f"{name}_llvm.exe"

        # A. Compile with Cranelift
        cl_comp = run_command(["lpp", str(path), "-o", str(cl_exe)], cwd=ROOT_DIR)
        if not cl_comp["success"]:
            print(f"{RED}FAILED (Cranelift Compile){RESET}")
            results.append({
                "name": name,
                "status": "FAIL_CRANELIFT_COMPILE",
                "error": cl_comp["stderr"] or cl_comp["stdout"],
            })
            failed_count += 1
            continue

        # B. Run Cranelift binary
        cl_run = run_command([str(cl_exe)], cwd=ROOT_DIR)
        if not cl_run["success"]:
            print(f"{RED}FAILED (Cranelift Runtime: code {cl_run['returncode']}){RESET}")
            results.append({
                "name": name,
                "status": "FAIL_CRANELIFT_RUNTIME",
                "error": f"Exit Code: {cl_run['returncode']}\nStdout: {cl_run['stdout']}\nStderr: {cl_run['stderr']}",
            })
            failed_count += 1
            continue

        # C. Compile with LLVM
        llvm_comp = run_command(["lpp", str(path), "--llvm", "-o", str(llvm_exe)], cwd=ROOT_DIR)
        if not llvm_comp["success"]:
            print(f"{YELLOW}CRANELIFT OK{RESET} | {RED}LLVM Compile FAIL{RESET}")
            results.append({
                "name": name,
                "status": "FAIL_LLVM_COMPILE",
                "error": llvm_comp["stderr"] or llvm_comp["stdout"],
            })
            failed_count += 1
            continue

        # D. Run LLVM binary
        llvm_run = run_command([str(llvm_exe)], cwd=ROOT_DIR)
        if not llvm_run["success"]:
            print(f"{YELLOW}CRANELIFT OK{RESET} | {RED}LLVM Runtime FAIL (code {llvm_run['returncode']}){RESET}")
            results.append({
                "name": name,
                "status": "FAIL_LLVM_RUNTIME",
                "error": f"Exit Code: {llvm_run['returncode']}\nStdout: {llvm_run['stdout']}\nStderr: {llvm_run['stderr']}",
            })
            failed_count += 1
            continue

        # E. Differential Verification: Cranelift vs LLVM stdout
        cl_out = cl_run["stdout"].strip().replace("\r\n", "\n")
        llvm_out = llvm_run["stdout"].strip().replace("\r\n", "\n")

        if cl_out != llvm_out:
            print(f"{RED}DIFFERENTIAL MISMATCH{RESET}")
            results.append({
                "name": name,
                "status": "FAIL_DIFFERENTIAL",
                "error": f"--- Cranelift Output ---\n{cl_out}\n--- LLVM Output ---\n{llvm_out}",
            })
            failed_count += 1
            continue

        # Success!
        print(f"{GREEN}PASSED (100% MATCH){RESET} [CL: {cl_comp['duration_ms']}ms | LLVM: {llvm_comp['duration_ms']}ms]")
        results.append({
            "name": name,
            "status": "PASS",
            "cl_comp_ms": cl_comp["duration_ms"],
            "llvm_comp_ms": llvm_comp["duration_ms"],
            "run_ms": cl_run["duration_ms"],
        })
        passed_count += 1

    # Print Summary Table
    print(f"\n{BOLD}{CYAN}=================================================================={RESET}")
    print(f"{BOLD}{CYAN}                    TEST EXECUTION SUMMARY                       {RESET}")
    print(f"{BOLD}{CYAN}=================================================================={RESET}\n")
    print(f"Total Tests Executed : {len(all_tests)}")
    print(f"Passed (100% Match)  : {GREEN}{passed_count}{RESET}")
    print(f"Failed               : {RED if failed_count > 0 else GREEN}{failed_count}{RESET}\n")

    if failed_count > 0:
        print(f"{BOLD}{RED}FAILED TESTS DETAIL:{RESET}")
        for r in results:
            if r["status"] != "PASS":
                print(f"\n{BOLD}[{r['name']}] Status: {r['status']}{RESET}")
                print(f"{r['error']}")
        sys.exit(1)
    else:
        print(f"{BOLD}{GREEN}ALL {passed_count} TEST SUITES PASSED WITH 100% DIFFERENTIAL FIDELITY!{RESET}\n")
        sys.exit(0)


if __name__ == "__main__":
    main()
