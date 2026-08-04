# L++ usage

**Current as of 2026-07-30.**

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

`clang` must be installed for the LLVM backend. Set `LPP_LLVM_CC` if it has a
non-standard path. `LPP_LLVM_MARCH=native` enables host LLVM CPU features.

## Package manager and versioning

```sh
lpp init demo
lpp version                         # show `demo` and its SemVer
lpp version set 1.2.3
lpp version bump patch              # 1.2.4
lpp add local-lib --path ../local-lib
lpp install --offline               # never consult the network
lpp workspace members
lpp workspace graph
```

The Rust package manager is the reliable default. The pure-L++ implementation
is available for experiments with `LPP_SELF_HOSTED_PM=1`; failures from either
implementation are returned as non-zero process statuses.

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

The executor is deterministic, single-threaded, and run-to-completion. It does
not claim nonblocking socket/file I/O or general coroutine suspension.

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
(cd packages/lppsqlite && sh run-tests.sh)
(cd packages/compresslpp && sh run-tests.sh)
```

Read [Current Capabilities](CURRENT_CAPABILITIES.md) for the supported feature
boundary and [STATUS-2026-07-30](STATUS-2026-07-30.md) for measured results.
