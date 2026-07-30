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
sh scripts/check_safety_mission.sh
(cd packages/lppsqlite && sh run-tests.sh)
(cd packages/compresslpp && sh run-tests.sh)
```

Read [Current Capabilities](CURRENT_CAPABILITIES.md) for the supported feature
boundary and [STATUS-2026-07-30](STATUS-2026-07-30.md) for measured results.
