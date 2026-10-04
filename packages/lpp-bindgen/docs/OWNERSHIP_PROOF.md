# Path-sensitive ownership proof

`mode: "ownership-proof"` upgrades the site-counting ownership graph
(`ownership-graph`) into a per-function, path-sensitive proof of allocation
ownership over a C translation unit. It walks each function body and classifies
every function that allocates, reallocates, or frees.

## Classification

| Verdict | Meaning |
| --- | --- |
| `proved-balanced` | Every allocation is freed on every path and none escapes to the caller. |
| `proved-escape` | The sole surviving allocation is returned to the caller. |
| `proved-leak` | A path returns (or falls off) with a live, non-returned allocation. |
| `proved-double-free` | An internal allocation is freed twice on a path. |
| `unproven` | Goto/switch/labels, divergent ownership across branches, or an untracked pattern. |

A function with no allocation/reallocation/deallocation sites is not reported.

## What it tracks

On the plain-local-variable-identity subset, the walker models:

- `v = alloc(...)` — `v` becomes owned;
- `v = realloc(v, ...)` — the old handle is consumed and `v` stays owned;
- `type *v = alloc(...)` — local declarator with an allocator initializer;
- `free(v)` — owned `v` becomes freed (a second `free(v)` is double-free);
- `return v` where `v` is owned — ownership transfers to the caller (escape);
- `return alloc(...)` — escape;
- overwriting an owned handle with a non-allocator value — the old allocation
  leaks.

Loops (`while`, `for`, `do`) must be ownership-neutral across one iteration to
be provable; otherwise they are `unproven`. A cast initializer such as
`(int *)calloc(...)` is deliberately not recognized and falls back to
`unproven` — the analysis is sound and conservative, never guessed.

## Example verdicts

From the `fixtures/ownership_proof.c` fixture:

```text
proof|balanced_path|proved-balanced|alloc=1|realloc=0|free=1
proof|escape_path|proved-escape|alloc=1|realloc=0|free=0
proof|leak_path|proved-leak|alloc=1|realloc=0|free=0
proof|double_free_path|proved-double-free|alloc=1|realloc=0|free=2
proof|divergent_path|unproven|alloc=1|realloc=0|free=1
proof|goto_path|unproven|alloc=1|realloc=0|free=1
proof|realloc_balanced|proved-balanced|alloc=1|realloc=1|free=1
```

## Implementation notes

- The walker returns integer source offsets and threads all state through a
  mutable byte buffer (owned-set string, freed-set string, flags word,
  allocation count). No hot-path function returns a `Str`-containing struct by
  value.
- The module deliberately avoids the six-argument `c_lex_cursor` reconstruction
  form and uses `c_lex_advance(cursor, offset)` instead. Both the 
  struct-with-`Str`-by-value pattern and the six-argument `c_lex_cursor` call
  were observed to destabilize the L++/Cranelift AOT backend in this large
  combined program (spurious `mismatched argument count` / `type i8, expected
  i64` verifier errors that pass `--check`). The module is written to steer
  clear of both.

## Output

- `c2lpp.ownership-proof.txt` — one `proof|name|verdict|alloc=..|realloc=..|free=..`
  record per allocating function;
- `c2lpp.ownership-proof-report.txt` — aggregate counts
  (`proved_balanced`, `proved_escape`, `proved_leak`,
  `proved_double_free`, `unproven`).
