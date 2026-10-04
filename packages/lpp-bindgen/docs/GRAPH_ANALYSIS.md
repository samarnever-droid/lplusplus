# c2lpp general call, control-target and ownership graphs

These passes operate on the same exact function spans produced by
`c_translation_unit.lpp`. They are structural prerequisites for translation;
they do not claim typed expression semantics or executable CFG completion.

## Call graph (`mode: "call-graph"`)

The call scanner walks every function body while respecting strings, character
literals, line comments, block comments and balanced braces. It emits one
`caller` record per function and one `call` record per detected site:

```text
caller|graph_analyze
call|graph_analyze|calloc|allocator|offset=...
call|graph_analyze|transform|indirect-or-unresolved|offset=...
call|graph_analyze|free|deallocator|offset=...
```

Resolution classes:

- `defined`: a function body exists in the translation unit;
- `declared`: a prototype exists but no body is present;
- `allocator`, `reallocator`, `deallocator`: known ownership operations;
- `indirect-member`: a call through `.` or `->` syntax;
- `indirect-or-unresolved`: no direct target can yet be proved.

Allocator classification is independent of direct/prototype resolution so a
standard allocation prototype still counts as an ownership site.

## Control-target graph (`mode: "control-graph"`)

Labels are function-scoped. The resolver records every ordinary label and every
`goto` target, then verifies:

- no duplicate ordinary label in one function;
- every direct goto target exists in that function;
- braces remain balanced;
- function denominator matches the TU graph.

It also inventories switch/case, conditions, loops, breaks, continues and
returns. Conservative block/edge counts are planning data for the later typed
CFG builder; they are not dominance or reducibility proofs.

Example:

```text
label|graph_analyze|failed|offset=...
edge|graph_analyze|goto|failed|offset=...|resolved=1
control|graph_analyze|valid=1|labels=1|gotos=1|resolved=1|...
```

## Ownership-site graph (`mode: "ownership-graph"`)

The ownership pass consumes call records in caller order and summarizes:

```text
ownership|function|alloc=N|realloc=N|free=N|classification
```

Classifications:

- `none`;
- `alloc-only`;
- `free-only`;
- `realloc-only`;
- `site-balanced-needs-path-proof`.

“Site-balanced” means both allocation and free sites exist. It does **not** mean
every execution path is balanced. Functions with any ownership operation are
explicitly counted as requiring path-sensitive CFG analysis.

## Cross-pass consistency (`mode: "graph-check"`)

The consistency pass reruns TU, declaration, body, call, control and ownership
analysis over one source and checks shared invariants:

1. all passes agree on function count;
2. declaration count equals the TU denominator;
3. no unknown top-level records;
4. no unresolved base-type family;
5. no unbalanced function span;
6. no duplicate/unresolved direct goto target;
7. call and ownership allocation/reallocation/free totals agree.

A successful report still contains:

```text
semantic_translation_complete=0
```

until typed AST, resolved CFG, code generation and execution gates pass.

## Complexity and memory policy

All three scanners use checked byte buffers and bounded growing output buffers.
They do not materialize complete per-token object lists or retain function-body
ASTs. This keeps whole-amalgamation inventory linear and avoids the earlier
memory failure caused by millions of managed token records.

## Production boundary

The graph suite improves observability and provides denominators for ownership
and CFG work. Remaining obligations include:

- exact function-pointer target sets;
- expression type and place resolution;
- path-sensitive allocation transfer;
- dominance, loop and switch edge construction;
- callback escape/lifetime policy;
- vararg call-frame lowering;
- generated pure-L++ execution.
