# 1. L++ Architecture Summary

L++ is a strongly typed, ahead-of-time (AOT) compiled language designed for safety and speed, relying on **ARC (Automatic Reference Counting)** with explicit lifetime and borrowing controls instead of a garbage collector or complex lifetime annotations. The compiler is written in Rust (`src/` with `main.rs`), and it has a custom direct linker (`lpp-link`).

The compilation pipeline is clean and linear:
1. **Frontend:** Lexer (`src/frontend/lexer.rs`) \u2192 Parser (`src/frontend/parser.rs`) \u2192 AST (`src/frontend/ast.rs`).
2. **Analysis:** Semantic Analysis (`src/analysis/semantic.rs`) \u2192 Type Checking (`src/analysis/typecheck.rs`) \u2192 Monomorphization (`src/analysis/monomorph.rs`) \u2192 Cycle Breaking (`src/analysis/cyclebreak.rs`). 
3. **MIR (Mid-level IR):** Lowers AST to MIR (`src/mir/lower.rs`).
4. **Safety & Optimization Passes:** Borrow validation (`validate_borrows.rs`), Escape solver (`escape_solver.rs`), ARC insertion (`pass_arc.rs`, `pass_arc_local.rs`), Inlining, Copy/Const propagation, Peephole, DCE.
5. **Backend:** Lowers MIR to Machine Code via Cranelift (`src/backend/cranelift/` default) or LLVM (`src/backend/llvm.rs`).
6. **Linking:** Uses system linker or `lpp-link` for creating native binaries (ELF/PE/Mach-O).

The **Runtime System** has been layered efficiently:
- `Layer 3 (stdlib/)`: Pure L++ modules (no C dependencies).
- `Layer 2 (runtime/)`: Platform runtime (`unified.c`, `windows_x86_64_min.c`, `linux_x86_64_min.c`). Builtins requiring OS interaction (I/O, networking).
- `Layer 1 (lpp-link)`: Direct binary emission without needing C code changes.

# 2. What is already strong

- **Fast Compilation:** Uses Cranelift emitting bounded batches; avoids holding the entire program's IR in memory. The compiler pipeline is well-separated.
- **Cycle Breaking (Memory Safety):** L++ implements an innovative static cycle breaker (`src/analysis/cyclebreak.rs`) which demotes `Owning` edges in recursive structs to `NonOwning`, preventing ARC leaks (cycles) statically while preserving trees/linked lists.
- **Safety Checks:** Explicit AST-to-MIR borrow validation (`validate_borrows.rs`) prevents returning slices, storing them in structs, or passing them to thread spawns/captures.
- **Platform Separation:** The C runtime isolates libc dependencies nicely from freestanding paths (useful for tiny 15KB PE executables).
- **Tooling:** Simple `cargo` build and a unified `lpp` CLI with built-in testing (`cargo test` suite has 60+ ownership tests).
- **Concurrency & FFI:** Built-in lightweight `spawn` and straightforward C interop (`extern "C"`).

# 3. What is currently weak

- **Borrowing Limitations:** Slice views are strictly "first-tier" and cannot cross boundaries. They can't be returned from functions, stored in structs, or captured. This simplifies the compiler but heavily restricts expressiveness compared to Rust's explicit lifetimes.
- **Generics Inference Limitations:** From the `FIXES.md`: "An empty list literal infers as `List[Int]`, so `children: List[Node]` cannot yet be populated inline". The type inference engine struggles with generic type variables on empty collections.
- **String Temporaries Leaking:** `FIXES.md` states: "String temporaries (`lpp_str_concat` results) still leak \u2014 a separate allocation path from struct ARC, unaffected by this work." String memory management appears slightly out-of-sync with struct ARC.
- **Runtime Error Diagnostics:** Panics or out-of-bounds access might crash or provide limited stack traces due to the minimalistic runtime and `lpp-link`.
- **WASM Backend:** Code shows recent work in `wasm.rs` with unused constants, suggesting this backend is in-progress and untested compared to Cranelift.
- **Ecosystem:** `pm.rs` exists, but the package manager is still tightly coupled to the compiler executable and relies on an internal registry file.

# 4. Important technical risks

- **ARC Cycle Breaking heuristic:** While statically proven to prevent leaks, the "heuristic" of which edge to demote could silently cause use-after-free bugs if generation checks or weak-reference validation are incorrectly timed (though recent fixes patched one such hole).
- **Single-Thread Executor constraints:** The async executor is single-threaded. Blocking calls without readiness adapters reject async call graphs. Expanding this to true multi-threading could violate existing assumptions in `validate_borrows.rs`.
- **Custom Linker (`lpp-link`):** Maintaining a custom linker for PE/ELF/Mach-O is extremely error-prone. A small change in OS loader requirements (e.g. Windows ASLR, codesigning) could break emitted binaries silently.
- **Cranelift changes:** Upgrading Cranelift (currently `0.113`) often breaks APIs. The backend is tightly coupled to this version.

# 5. Current test/benchmark state

- **Unit/Integration Tests:** Extensive tests in `tests/*.lpp` covering loops, branches, memory (hard memory stress), ownership regressions, and closures.
- `FIXES.md` reports 63 Cargo tests passing and `aot_parity.tsv` passing 34/34 checks.
- **Benchmarks:** The BPW v3 (benchmarks/king20) lists extremely fast CPU/RAM heavy test executions (4ms L++ vs 3ms Rust vs 5ms Go) and tiny binary sizes (15.5KB Win PE).
- **Failing WSL Executions:** Currently `cargo` is missing from the Arch WSL environment which prevents `run_aot_parity.sh` from being run directly inside WSL by the agent. However, tests passed cleanly on Windows host (`cargo check`).

# 6. Highest-priority TODO (Critical)

1. **Fix String Temporary Leaks:** Unify the string allocation/deallocation path with the standard ARC pass.
   - *Why:* Prevents long-running network/file-processing applications from OOMing over time.
   - *Affected files:* `src/mir/pass_arc.rs`, `lpp_runtime.c` (`lpp_str_concat`).
   - *Difficulty:* Medium.
2. **Fix Type Inference for Empty Collections:** Allow `mut lst = list_new()` to defer type resolution until the first push or allow explicit generic turbofish on the call (e.g. `list_new[Node]()`).
   - *Why:* Unblocks ergonomic construction of Trees and Graphs.
   - *Affected files:* `src/analysis/typecheck.rs`, `src/frontend/parser.rs`.
   - *Difficulty:* Medium.
3. **Robustness of WASM & LLVM Backends:** Clean up unused imports/variables in `wasm.rs` and stabilize the backend interfaces so they match Cranelift's capabilities.

# 7. Medium/long-term TODO

1. **Advanced Borrowing (Lifetimes):** Introduce basic lifetime parameters so views/slices can be returned or stored in short-lived structs.
   - *Difficulty:* High.
2. **Multi-threaded Async Runtime:** Extend the single-thread async executor to use a work-stealing thread pool, updating `validate_borrows.rs` to allow `Send`-able tasks across boundaries.
   - *Difficulty:* Very High.
3. **Ecosystem Tooling:** Extract the package manager (`pm.rs`) into a standalone cargo-like tool (`lpp-pm` or similar) to separate build-tooling from compiler logic.

# 8. Proposed L++ target

**Target for Next Major Version (v5.0): The "Systems Ready" Release**
- **Memory Safety:** 100% leak-free ARC (including all string temporaries).
- **Language Completeness:** Ergonomic generic type inference (no more `List[Int]` defaults for empty lists). Full support for explicit interface/trait implementations.
- **Cross-Platform:** Perfect parity between Windows PE (direct link) and Linux ELF targets.
- **Tooling:** A standalone language server (`lpp-lsp`) that offers reliable auto-complete and go-to-definition, and a unified package manager.
- **Compiler Speed:** Maintain sub-10ms compilation for standard projects; keep memory usage low via Cranelift batched lowering.

# 9. Recommended implementation order

1. **Investigate String Leaks:** Write an explicit test in `tests/` verifying `str_concat` leaks. Patch `pass_arc.rs` or `lpp_str.c` to track string temporaries properly.
2. **Type Inference Fix:** Modify `typecheck.rs` to support delayed type resolution or explicit type parameters for `list_new()`.
3. **WASM Backend Cleanup:** Run `cargo fix` to remove dead code in `wasm.rs`.
4. **Standard Library Expansion:** Port more basic algorithms to pure `stdlib/` to prove the layered architecture works.

# 10. Specific files/subsystems to change first

- `src/analysis/typecheck.rs` (for empty list type inference)
- `src/mir/pass_arc.rs` (for string temporaries)
- `runtime/lpp_str.c` & `runtime/unified.c` (to ensure ARC headers match string allocation)
- `src/backend/wasm.rs` (cleanup warnings and unused constants)
