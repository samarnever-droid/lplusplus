# L++ usage

**Current as of 2026-10-26.**

## Compile and run

```sh
lpp app.lpp
./app
```

The default backend is Cranelift. Select LLVM explicitly:

```sh
lpp app.lpp --backend llvm
lpp app.lpp --backend llvm --linker direct
```

Emit an object without linking:

```sh
lpp app.lpp --backend llvm --emit-object
```

An LLVM-compatible compiler must be installed for the LLVM backend. Set it
persistently with `lpp config set llvm-path /path/to/clang`, or use
`LPP_LLVM_CC=/path/to/clang` as a per-process override.
`LPP_LLVM_MARCH=native` enables host LLVM CPU features in the compatibility
backend.

## Native optimization and PGO

```sh
lpp app.lpp -O3 --lto -o app-fast
./app-fast
```

`-O0` disables Cranelift post-optimization; `-O1`/`-O2`/`-O3` select its speed
pipeline, while `-Os`/`-Oz` select speed-and-size. `--lto` enables a bounded
whole-program MIR inliner for tiny, direct, pure scalar callees. It is not
object-file or foreign-library LTO.

Profile-guided hot-callee inlining uses a two-step workflow:

```sh
lpp app.lpp -O3 --pgo-generate -o app-instrumented
LPP_PGO_PROFILE=app.pgo ./app-instrumented
lpp app.lpp -O3 --pgo-use app.pgo -o app-pgo
./app-pgo
```

`--pgo-generate` currently requires Cranelift and the compiler host target. The
runtime writes `lpp.pgo` if `LPP_PGO_PROFILE` is unset; profile data is flushed
when source `main` returns normally. `--pgo-use` checks the profile's program
fingerprint before using function-entry counts to select hot callees for the
same bounded MIR inliner. It does not guide backend machine-code optimization.

## Package manager and versioning

```sh
lpp init demo
lpp version                         # show `demo` and its SemVer
lpp version set 1.2.3
lpp version bump patch              # 1.2.4
lpp add local-lib --path ../local-lib
lpp install --offline               # never consult the network
lpp config set backend cranelift
lpp config set linker auto
lpp workspace members
lpp workspace graph
```

The Rust package manager is the reliable default. The pure-L++ implementation
is available for experiments with `LPP_SELF_HOSTED_PM=1`; failures from either
implementation are returned as non-zero process statuses.

Use `lpp upgrade --check` to inspect the release channel. Automatic binary
replacement is intentionally disabled: download the release archive and
`SHA256SUMS`, verify the digest, and then run the installer.

## Debugging and inspection

| Flag | Purpose |
|---|---|
| `--dump-ast` | Print the parsed AST |
| `--dump-symbols` | Print semantic bindings/scopes |
| `--dump-types` | Print inferred types |
| `--dump-escape` | Print MIR Frame/Owned/Shared facts and stack-promotion summary |
| `--dump-mir` | Print lowered/cleaned MIR |
| `--check` | Frontend/type-check without native linking |
| `--checkall` | Check all project `.lpp` files |

There is no C-transpiler dump and no Turbo mode in the current repository.

## Experimental tuple/rest/slice/task syntax

```lpp
def values(label: Str, ...items: Int) -> (Str, Int):
    return (label, list_len(items))

async def get_label() -> Str:
    return "Indore"

async def main():
    label := get_label().await
    (name, count) := values(label, 1, 2, 3)
    view := str_slice(name, 0, 3)
    print(count)
    print(str_slice_to_str(view))
```

Rules in the current first tier:

- tuple arity is 2–4 and `(expr)` remains grouping;
- `...items: T` must be the final parameter; the body sees `List[T]`;
- extern functions reject variadic syntax;
- `str_slice`/`slice` are zero-copy borrowed views; `slice_len` and
  `slice_get` are checked operations;
- a borrowed view cannot return, be captured/stored, cross `spawn`, reach an
  unknown retaining call, or survive source reassignment;
- `str_slice_to_str` (also `slice_to_str`) explicitly allocates an owned `Str`;
- `.await` is legal inside `async def`; async `main` is driven by the executor;
- task values are single-executor confined and cannot be captured by closures in
  this tier;
- blocking input/file/network/process calls are rejected transitively from
  async call graphs.

`async def` tasks are dispatched to a multi-threaded worker pool (`2..=16` OS
worker threads on hosted native targets) with cooperative caller stealing on
`.await`. An OS readiness reactor (`epoll`/`kqueue`/`IOCP`) for nonblocking
socket/file I/O is not yet integrated.

## Linkers

```sh
lpp app.lpp --linker host
lpp app.lpp --linker direct
```

The host path uses the platform C linker/runtime. The direct path uses
`lpp-link` with the platform freestanding runtime for its verified target
subset.

## Environment

- `BENCHMARK=1`: print timing JSON;
- `LPP_AOT=1`: emit native object mode;
- `LPP_LLVM_CC=/path/to/clang`: LLVM compiler executable;
- `LPP_LLVM_MARCH=native`: request LLVM host CPU features;
- `LPP_CRANELIFT_SIMD=0`: disable Cranelift host SIMD feature selection.

## Validation commands

```sh
cargo test --release -j1
sh tests/run_aot_parity.sh
LPP_LLVM_CC=/path/to/clang sh tests/run_llvm_smoke.sh
LPP_LLVM_CC=/path/to/clang sh tests/run_feature_batch.sh
sh scripts/check_safety_mission.sh
sh tests/test_pm_workflow.sh
(cd packages/lppsqlite && sh run-tests.sh)
(cd packages/compresslpp && sh run-tests.sh)
```

Read [Current Capabilities](CURRENT_CAPABILITIES.md) for the supported feature
boundary and [v0.3.2 Release Notes](release/V0.3.2.md) for current highlights.
