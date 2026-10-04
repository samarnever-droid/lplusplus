# L++ Android & Termux support

L++ can target Android and Termux. Termux is a full Linux userspace that runs on
Android's kernel with the bionic libc; a Termux build of L++ is therefore an
`aarch64` (or `armv7`) Linux build. Android itself uses the same ELF format and
the bionic libc, so the same toolchain can produce Android-NDK-friendly binaries.

## Native Termux builds (run on the device)

Termux ships a working `cc`/`clang` and linker, so building L++ and running
programs on a Termux device is just a normal aarch64-Linux build:

```sh
# on the device, in a Termux terminal
pkg install binutils build-essential cargo rust clang
cargo build --release --bin lpp --bin lpp-link
./target/release/lpp hello.lpp          # compiles + links via the host cc
./hello
```

Because Termux provides a full libc, `print`/`input`/file I/O work in the
terminal as on any Linux. The runtime uses `printf`/`stdout`, so console output
appears normally in the Termux shell.

## Cross-compiling for Android / Termux from a desktop

`--target <triple>` selects the output architecture/OS. The Cranelift backend
selects the matching ISA (aarch64, armv7, x86, riscv64) when the compiler is
built with the `all-arch` feature (the default).

```sh
# Emit an AArch64 ELF object for Android arm64 / Termux 64-bit
./target/release/lpp app.lpp --target aarch64-linux-android --emit-object
# => app.o is an AArch64 ELF object (check: readelf -h app.o)

# Emit for Android arm32
./target/release/lpp app.lpp --target armv7-linux-androideabi --emit-object

# List the supported triples
./target/release/lpp --list-targets
```

The object can then be linked with the Android NDK:

```sh
# with ANDROID_NDK_HOME set, `--linker host` uses the NDK clang automatically
export ANDROID_NDK_HOME=/path/to/android-ndk
./target/release/lpp app.lpp --target aarch64-linux-android --linker host
```

When `ANDROID_NDK_HOME` (or `ANDROID_NDK_ROOT`) is set, L++ uses the NDK's clang
(`-target <triple>` + `-DLPP_ANDROID`) and links `-llog`. Set `ANDROID_CC` or
`LPP_CC` to override the C compiler explicitly.

## Target-triple table

| Triple | Platform |
| --- | --- |
| `aarch64-linux-android` | Android arm64 / Termux 64-bit |
| `armv7-linux-androideabi` | Android arm32 |
| `arm-linux-androideabi` | Android arm (v7) |
| `i686-linux-android` | Android x86 |
| `x86_64-linux-android` | Android x86_64 |
| `aarch64-unknown-linux-gnu` | generic arm64 Linux |
| `x86_64-unknown-linux-gnu` | generic x86_64 Linux |
| `riscv64gc-unknown-linux-gnu` | generic riscv64 Linux |

## Runtime notes

- `lpp_runtime.c` is compiled for the selected target. Android builds use
  `-DLPP_ANDROID`, which routes `lpp_print_str` to `__android_log_print`
  (visible in `logcat`); Termux and host builds keep normal `printf`/`stdout`.
- The runtime cache key includes the target, so Android and host runtimes never
  collide.
- For Android *apps* (no console) the entry point is the same executable; output
  goes to logcat when `-DLPP_ANDROID` is active. For Termux (a console), output
  goes to the terminal.

## Tests

`tests/test_target_android.sh` (run by `tests/run_target_tests.sh`, and wired
into CI) verifies:

1. `--list-targets` advertises the Android/Termux triples;
2. `--target aarch64-linux-android --emit-object` produces a real AArch64 ELF
   object;
3. a host target still builds and runs.

## Requirements

- Cranelift backend compiled with the target arch feature. The repo default is
  `all-arch` (all architectures). A build restricted to `x86` only emits x86
  targets.
- To link an Android object, either set `ANDROID_NDK_HOME`/`ANDROID_NDK_ROOT`
  (or `ANDROID_CC`/`LPP_CC`) to an NDK clang, or use a cross `cc` directly.
  Termux devices cross-link with their own `cc`.
