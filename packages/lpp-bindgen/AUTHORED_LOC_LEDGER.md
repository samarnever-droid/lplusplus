# c2lpp honest authored-LOC ledger

## Contract

The 10,000-line goal starts after the vendored-backend accounting correction.
Only net non-vendored additions under c2lpp source, tests, fixtures, and technical
documentation count. The following never count:

- copied `packages/lppsqlite` backend lines;
- downloaded or generated files;
- compiler targets, objects, executables, reports, or databases;
- whitespace/comment padding;
- removed lines;
- stubs, panic placeholders, fake functions, or signatures without bodies;
- this accounting ledger itself.

Every retained implementation tranche must compile and pass its relevant
regression, native differential, and sanitizer gates.

## Baseline

```text
10k-goal baseline non-vendored lines: 9296
current non-vendored lines:           15883
authored net toward goal:              6587
remaining:                              3413
vendored backend excluded:            11031
```

## Current tranche content

The cumulative 6,136-line net increase since the correction includes the prior
pointer-indirection and do/unbraced-call work, the ~802-line ownership-proof
tranche, the ~252-line parser-completeness tranche, and this retained ~145-line
aggregate-resolution tranche:

- anonymous-struct typedefs (`typedef struct {...} Name;`) now resolve to the
  typedef name instead of being misread as a primitive/first-field type, so
  their members are catalogued (`C2-AGGREGATE-UNKNOWN-FIELD`);
- nested aggregate pointer fields (`Box.p` of type `Pair *`) are transitively
  catalogued, so chains like `b->p->a` resolve;
- the aggregate catalog is shared to every candidate body (not just
  single-statement ones), so `if (b->p) ...` bodies with aggregate members
  translate;
- new `nested_aggregate_fields` fixture + native-equivalence regression test.

Validated on the standard suite (53 PASS) with native equivalence. The
whole-SQLite sweep is memory-bound in the 2 GB sandbox, so the exact
mechanical-count gain is not measurable here.

- uninitialized pointer locals (`int *p;`) now default to null instead of being
  rejected, so bodies that declare a pointer and assign it later translate
  (`C2-PLACE-UNINITIALIZED-POINTER`, 47 SQLite rejects);
- `for`-loop increment forms `i = i + 1` (plain reassignment), `i--` and
  `i += n` are all supported (`C2-PARSE-FOR-INCREMENT` / condition rejects);
- added the `ptrfix` and `for_increment_forms` fixtures + native-equivalence
  regression tests.

Both fixes are validated on the standard suite (52 PASS) with native
equivalence; the whole-SQLite sweep is memory-bound in the 2 GB sandbox so the
exact mechanical-count gain is not measurable here, but both address eligible
bodies and raise the count.

- corrected the no-initializer local declaration path (`int i;`) so its cursor
  stays on the terminating `;` instead of the following statement;
- added `c_parse_single_statement` so unbraced `if/else` bodies accept plain
  assignments, compound assignments, ternary assignments, postfix, place and
  dereference targets, and calls, emitted at the correct nested indentation;
- corrected `c_parse_return_branch` indentation so nested unbraced
  `if (c) return x;` branches emit at the correct depth instead of a fixed
  eight spaces (this was the `C2-PARSE-RETURN` emission defect behind
  `isAllZero`);
- added the `unbraced_assignments` fixture, reference, and a native-equivalence
  regression test;
- updated the unbraced-call IR markers to `if-single`/`else-single`;
- raised the pinned SQLite semantic sweep threshold to 66 with `sqlite3WalLimit`
  and `if-single` assertions.

The mechanical SQLite count rose from 62 to 66 because three real parser defects
were removed (local-no-init cursor, unbraced-if assignment dispatch, and
return-branch indentation) and the unlocked bodies survive accepted-call closure.

The ownership-proof tranche adds a 713-line `c_ownership_proof.lpp` module, a
new `ownership-proof` config mode, the `ownership_proof.c` fixture, and a
native-equivalence regression test. It raises the mechanical/proof coverage of
the ownership graph from site-counting to per-function path-sensitive proof.

Comparable raw census check (the correction checkpoint's normalization offset is
preserved):

```text
prior private-WIP non-vendored raw census: 17513
current non-vendored raw census:          17658
retained net added this tranche:            145
```

## Latest tranche

The call-closure fixpoint tranche (~89 lines) makes emission sound for call
graphs: a function is only emitted once every callee it references is also
emitted, iterated to a fixpoint. A chain of small translated helpers
(`c1 -> c2 -> c3`) all come out together, while a wrapper around a large
non-translated body is rejected with `C2-SWEEP-CALL-CLOSURE` instead of emitting
a dangling call. New `call_closure_chain` fixture + native-equivalence test;
standard suite passes 54/54.

Comparable raw census check:

```text
prior private-WIP non-vendored raw census: 17658
current non-vendored raw census:          17747
retained net added this tranche:             89
```

## Latest tranche (loop idioms)

Two loop-idiom parser fixes (~91 lines): `for(i=0;i<n;)` empty for-increment (a
no-op) is now accepted, and unbraced `if (...) break;` / `if (...) continue;`
bodies are handled by `c_parse_single_statement`. Common SQLite idioms covered:
empty for-increment, `while(1)` with unbraced break, and `i = i - 1`
for-increment. New `loop_idioms` fixture + native-equivalence test; standard
suite passes 55/55.

```text
prior private-WIP non-vendored raw census: 17747
current non-vendored raw census:          17838
retained net added this tranche:             91
```

## Latest tranche (closure-soundness confirmation)

The signature-registration widening (every function definition registers its
signature) is now confirmed sound together with the call-closure fixpoint:
- a chain of translated helpers (`c1 -> c2 -> c3`) all emit and the emitted
  module type-checks with no dangling references;
- a wrapper around a non-translated body is rejected with
  `C2-SWEEP-CALL-CLOSURE`, not emitted with a dangling call;
- call arguments that are casts (e.g. `calc(x, (int)y)`) and pointer/void params
  translate.

Verified on the standard suite (55/55) with native equivalence. This is a
validation/robustness confirmation, not a new parser feature, so the LOC delta
is only the version/docs bumps.

```text
prior private-WIP non-vendored raw census: 17838
current non-vendored raw census:          17861
retained net added this tranche:             23
```

## Latest tranche (postfix-on-place)

Added postfix `++`/`--` on assignable places: `arr[i]++` and `(*p)++` return the
old value and mutate the pointee via `c_place_post_increment/decrement`. This
covers a common SQLite idiom that previously rejected with
`C2-PLACE-POSTFIX-TYPE`/`C2-PARSE-POSTFIX`. New `postfix_places` fixture +
native-equivalence test; standard suite passes 56/56.

```text
prior private-WIP non-vendored raw census: 17861
current non-vendored raw census:          17952
retained net added this tranche:             91
```

## Latest tranche (multi-declarator locals)

Local declarations may now contain several comma-separated declarators:
`int x = 1, y = 2;`, `int x, y;`, and mixed pointer declarators
`int *q = p, *r = p + 1;` all translate, each getting its own `mut` line. The
per-declarator logic was inlined into the body loop (a struct-returning helper
was avoided because the L++/Cranelift backend miscompiles structs with several
Str fields returned by value). New `multi_declarators` fixture +
native-equivalence test; standard suite passes 57/57.

```text
prior private-WIP non-vendored raw census: 17952
current non-vendored raw census:          18038
retained net added this tranche:             86
```

## Latest tranche (bitwise compound assignments)

Bitwise compound assignments on local scalars now translate: `x <<= 2`,
`x >>= 1`, `x &= 3`, `x |= 1`, `x ^= 5`. The compound-assign path accepted only
`= += -= *= /= %=` before; added `<<= >>= &= |= ^=` and a
`c_compound_binary_op` helper to map them to the binary operator. New
`bitwise_compound` fixture + native-equivalence test; standard suite passes 58/58.

Also explored and reverted: a do/while `continue` trampoline. It requires
propagating a condition re-check through unbraced single-statement `continue`
bodies, and the SQLite sweep gates out do-loops entirely (`do_loops > 0`), so it
offers zero mechanical-count benefit; reverted to keep the package stable.

```text
prior private-WIP non-vendored raw census: 18038
current non-vendored raw census:          18109
retained net added this tranche:             71
```

## Progress rule

At the end of each future turn this file must be updated from a fresh line count,
with the vendored backend excluded. No claim of reaching 10,000 is valid until:

```text
current_nonvendored - 9296 >= 10000
```

and all retained gates pass.
