import os
import sys
import glob
import subprocess
import time

def main():
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    os.chdir(root)
    
    files = sorted(glob.glob('tests/**/*.lpp', recursive=True))
    print(f"================================================================")
    print(f"   L++ COMPREHENSIVE SUITE RUNNER: {len(files)} TEST FILES")
    print(f"================================================================")
    
    passed = 0
    failed = 0
    skipped = 0
    failures = []
    
    # Submodules and helper files (tested through test_modules.lpp)
    module_files = {
        os.path.normpath("tests/modules/math_utils.lpp"),
        os.path.normpath("tests/modules/subpkg/deep_math.lpp"),
        os.path.normpath("tests/modules/subpkg/sub_helper.lpp"),
        os.path.normpath("tests/modules/text/string_ops.lpp"),
        os.path.normpath("tests/modules/greet.lpp"),
        os.path.normpath("tests/modules/math.lpp"),
        os.path.normpath("tests/modules/utils/helpers.lpp"),
    }
    
    for idx, f in enumerate(files, 1):
        norm_f = os.path.normpath(f)
        if norm_f in module_files:
            skipped += 1
            continue
            
        is_wasm_reject = "wasm" in f and "reject" in f
        is_negative_test = any(neg in os.path.basename(f).lower() for neg in ["reject", "bad", "fail", "invalid", "err", "cycle_rejected"]) or is_wasm_reject
        
        exe_name = f"test_run_{idx}.exe"
        wasm_name = f"test_run_{idx}.wasm"
        try:
            if is_wasm_reject:
                compile_cmd = ["lpp", f, "--target", "wasm32", "-o", wasm_name]
            else:
                compile_cmd = ["lpp", f, "-o", exe_name]
                
            res = subprocess.run(compile_cmd, capture_output=True, text=True, timeout=10)
            
            if is_negative_test:
                if res.returncode != 0:
                    print(f"[{idx:03d}/{len(files):03d}] PASS (Expected reject): {f}")
                    passed += 1
                else:
                    print(f"[{idx:03d}/{len(files):03d}] FAIL (Should have rejected): {f}")
                    failed += 1
                    failures.append((f, "Expected compilation failure but succeeded"))
            else:
                if res.returncode != 0:
                    print(f"[{idx:03d}/{len(files):03d}] FAIL (Compile error): {f}")
                    print(f"       {res.stderr.strip() or res.stdout.strip()}")
                    failed += 1
                    failures.append((f, f"Compile error: {res.stderr.strip()}"))
                else:
                    # Provide empty input to tests requiring stdin
                    stdin_data = "test\n" if "input" in f else ""
                    run_res = subprocess.run([os.path.join(".", exe_name)], input=stdin_data, capture_output=True, text=True, timeout=5)
                    if run_res.returncode != 0 and not "panic" in f:
                        print(f"[{idx:03d}/{len(files):03d}] FAIL (Runtime crash rc={run_res.returncode}): {f}")
                        print(f"       {run_res.stderr.strip()}")
                        failed += 1
                        failures.append((f, f"Runtime crash: {run_res.returncode}"))
                    else:
                        print(f"[{idx:03d}/{len(files):03d}] PASS: {f}")
                        passed += 1
        except Exception as e:
            print(f"[{idx:03d}/{len(files):03d}] ERROR: {f} -> {e}")
            failed += 1
            failures.append((f, str(e)))
        finally:
            for clean_target in [exe_name, wasm_name]:
                if os.path.exists(clean_target):
                    try:
                        os.remove(clean_target)
                    except:
                        pass

    print(f"\n================================================================")
    print(f"   SUMMARY: {passed} PASSED | {failed} FAILED | {skipped} SKIPPED")
    print(f"================================================================")
    
    if failures:
        print("\nFailures:")
        for fn, msg in failures:
            print(f" - {fn}: {msg}")
        sys.exit(1)
    else:
        print("\nALL TESTS IN TEST DIRECTORY PASSED 100%!")
        sys.exit(0)

if __name__ == "__main__":
    main()
