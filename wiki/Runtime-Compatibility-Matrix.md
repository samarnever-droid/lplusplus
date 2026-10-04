# Runtime Compatibility Matrix

L++ has a **host runtime** and smaller **freestanding runtimes** used by the direct linker.

This matters because a builtin can exist in `src/builtins.rs` but still need a platform runtime implementation to work with direct linking.

## Linker/runtime paths

| Path | Runtime | Typical use |
|---|---|---|
| Host linker | `lpp_runtime.c` plus `runtime/*.c` | full libc/CRT compatibility |
| Linux direct | `runtime/linux_x86_64_min.c` | freestanding ELF, no libc |
| Windows direct | `runtime/windows_x86_64_min.c` | freestanding PE, Kernel32-only |
| macOS host/direct | Mach-O path | currently less complete than Linux/Windows |

## Builtin category support

| Category | Host linker | Linux direct | Windows direct | Notes |
|---|---:|---:|---:|---|
| integer print | yes | yes | yes | `print`, `lpp_print_int` |
| string print | yes | yes | yes | `print_str` |
| float print | yes | partial | partial | `lpp_print_float`; public alias still needed |
| strings basic | yes | yes | partial/varies | `str_len`, `str_concat`, `str_substr`, `str_repeat` |
| strings extended | yes | yes | partial/varies | `char_at`, `ord`, `chr`, `str_find`, case conversion |
| lists | yes | yes | yes/core | list operations are core runtime functions |
| maps | yes | yes/core | yes/core | string-key behavior depends on runtime implementation |
| files | yes | partial | partial | direct runtime paths may not cover every helper |
| directories | yes | partial | partial | host path safest |
| process/env | yes | limited | limited | `command_exec`, `env_get`, `env_set` |
| buffers | yes | yes/recent | partial/limited | needed by ZIP package |
| networking | yes | limited/direct TBD | limited/direct TBD | host runtime recommended |
| HTTP | yes | limited/direct TBD | limited/direct TBD | host runtime recommended |
| JSON | yes | host-oriented | host-oriented | host runtime recommended |
| ZIP package | host recommended | experimental | experimental/usually skipped | uses `buf_*` heavily |

## Rule of thumb

- **Language features** usually do not need runtime or linker changes.
- **Pure computation builtins** can be implemented in L++ stdlib.
- **OS builtins** need runtime implementations.
- **New executable formats or dynamic linking behavior** are the only normal reasons to edit `lpp-link`.

## What to do when a direct-link build fails

If direct linking reports an unresolved symbol such as:

```text
unresolved external COFF symbol 'lpp_buf_len'
```

that usually means:

1. the compiler knows the builtin,
2. the host runtime may implement it,
3. but the freestanding runtime for that target does not yet implement it.

Fix the runtime C file, not the linker, unless the object/executable format itself is wrong.
