# L++ Safety Mission

## Promise and scope

L++ is pursuing Rust-like *memory-safety outcomes* for the supported language and platform subset. This is a verification mission, not a slogan. L++ must not claim full Rust-equivalent safety until every listed proof obligation has automated evidence across all supported execution paths.

The current public promise is narrower:

> In the verified Cranelift AOT subset, L++ rejects ownership operations it cannot lower with defined ARC/borrow semantics instead of silently producing a native executable.

## Safety levels

| Level | Meaning | Release rule |
|---|---|---|
| S0 — Prototype | Parser/backend feature exists; safety is unknown | Never describe as safe |
| S1 — Rejected safely | Unsupported ownership behavior has a deterministic diagnostic | Required before exposing incomplete syntax |
| S2 — Verified subset | MIR ownership model, runtime ABI, negative tests, parity tests, and platform scope agree | May be documented as supported subset |
| S3 — Cross-platform verified | S2 evidence exists for every advertised OS/architecture/link path | Required for default portable feature claims |
| S4 — Rust-equivalent claim | Formalized soundness boundary, adversarial/fuzz testing, no unsafe escape in user language, and independent audit | Not yet claimed |

## Non-negotiable invariants

1. User source has no raw pointer, use-after-free, or manual memory-free escape hatch.
2. Every owned AOT allocation has exactly one ownership root or a defined retain edge.
3. Every control-flow exit releases definite-live owned values exactly once.
4. Borrowed results never create an unretained owner escape.
5. Struct/list/closure destructors release owned children before storage reclamation.
6. Strong cycles are rejected until weak/arena/cycle-collector semantics are specified and tested.
7. An unsupported backend/runtime combination fails before executable output is produced.
8. Runtime ABI allocations are released by their matching allocator; adapters must copy across allocators where necessary.
9. Direct linkers reject unknown relocations/imports rather than emitting ambiguous binaries.
10. Benchmark, website, wiki, and README claims must not exceed S2 evidence.

## Current verified boundary

S2 applies only to the documented Linux x86-64 Cranelift AOT ownership subset: ARC structs, moves/borrows/owned returns, aliases, closure capsules, supported lists, destructor chains, and strong-cycle rejection. It does **not** automatically extend to C compatibility code, unimplemented thread transfer, direct networking linkage, all platform linkers, or future async tasks.

## Required evidence for new language features

A feature cannot graduate from S1 to S2 without all of:

- source-level contract and error behavior
- semantic/type checker validation
- ownership-aware MIR lowering
- ARC/runtime ABI implementation
- success tests and adversarial negative tests
- host-link AOT execution test
- backend parity test where C compatibility is supported
- documented platform/linker boundary
- benchmark only after correctness gates pass

## Mission workstreams

1. **Ownership completeness:** audit every MIR instruction and terminator for ownership transfer/cleanup.
2. **Boundary hardening:** remove or isolate runtime allocator/FFI ambiguity; version ABI headers.
3. **Differential testing:** run generated ownership programs through AOT and compatibility paths, compare output and rejection behavior.
4. **Adversarial testing:** cycles, aliases, branch exits, destructor nesting, closure captures, invalid handles, malformed objects.
5. **Platform evidence:** each public platform claim must have CI results, not host assumptions.
6. **Claim discipline:** safety wording is mechanically checked and release-blocking.

## What must happen before “as safe as Rust” can be written

- Complete language specification for lifetimes, aliasing, concurrency, panic/error unwinding, FFI, and unsafe boundaries.
- A formal or machine-checked soundness argument for the ownership core.
- Property/fuzz testing of parser, MIR lowering, ARC insertion, object writer, and runtimes.
- Cross-platform S3 evidence for every advertised default.
- Independent security review.

Until then, L++ uses the accurate phrase **ownership-aware, safety-oriented native language with a verified AOT subset**.
