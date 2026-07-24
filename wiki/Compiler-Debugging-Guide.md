# Compiler Debugging Guide

L++ exposes multiple debug dumps from the compiler pipeline.

## Basic checks

Check one file:

```bash
lpp --check file.lpp
lpp check file.lpp
```

Check a directory of `.lpp` files:

```bash
lpp --checkall
```

Important: the repository contains old negative tests and stale scratch files. Use a clean example directory for documentation checks.

## Frontend dumps

```bash
lpp --dump-ast file.lpp
lpp --dump-symbols file.lpp
lpp --dump-types file.lpp
```

Use these when debugging:

- parsing problems
- name resolution
- wrong inferred types
- generic/type-param behavior

## Ownership and MIR dumps

```bash
lpp --dump-escape file.lpp
lpp --dump-mir file.lpp
```

Use these when debugging:

- ARC behavior
- unexpected heap/stack classification
- closure capture
- match lowering
- `?` operator lowering
- short-circuit control flow

## Emit object only

```bash
LPP_AOT=1 lpp file.lpp
# or
lpp emit file.lpp --aot
```

This produces `.o` on Linux/macOS and `.obj` on Windows.

## Inspect native objects

`lpp-link` has an inspect mode:

```bash
lpp-link inspect file.o
lpp-link inspect file.obj
```

This is useful for linker/runtime bugs:

- missing symbols
- wrong section layout
- COFF relocation debugging
- direct PE failures

## Direct linker commands

```bash
lpp-link input.o runtime.o -o app
lpp-link pe input.obj runtime.obj -o app.exe
lpp-link macho input.o runtime.o -o app
```

## Benchmark/timing mode

The compiler supports benchmark-oriented timing output through environment variables used by CI:

```bash
BENCHMARK=1 LPP_AOT=1 lpp file.lpp
```

This is used by benchmark workflows to collect phase timings.
