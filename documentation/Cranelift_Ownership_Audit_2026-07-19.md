# Cranelift Ownership Correctness Audit

**Audit date:** 2026-07-19  
**Scope:** Cranelift AOT ownership pipeline, runtime ARC behavior, C/AOT observable parity, and ownership regression coverage.

## 1. Audit rule

No new ownership feature should be added unless the currently supported ownership contract passes the regression suite and known safety boundaries are explicit.

The audited AOT contract is:

```text
Custom struct / List[Int] / closure capsule local
    → owned ARC object

Function parameter of managed type
    → borrowed

Return of managed type
    → owned by caller

Direct alias or field alias
    → retain before second owner exists

Move
    → transfer owner without retain

Final release
    → type-specific destructor, then recursive child release
```

## 2. Edge cases audited

| Case | Regression source | Result |
|---|---|---|
| Scalar arithmetic and calls | `arith.lpp`, `nested_calls.lpp` | Pass |
| Branches / loops / recursion | `branches.lpp`, `loop.lpp`, `fib.lpp` | Pass |
| Basic immutable closure | `closure_test.lpp` | Pass |
| ARC struct return | `owned_return.lpp` | Pass |
| Branch-specific owned returns | `arc_branch_return.lpp` | Pass |
| Nested struct destructor chain | `arc_nested_struct.lpp` | Pass |
| Direct aliases | `arc_direct_alias.lpp` | Pass |
| Captured struct environment | `arc_closure_capture.lpp` | Pass |
| Borrowed parameter returned owned | `arc_borrowed_return.lpp` | Pass |
| Field aliases | `arc_field_alias.lpp` | Pass |
| Borrowed field returned owned | `arc_borrowed_field_return.lpp` | Pass |
| List[Int] alias / automatic cleanup | `arc_list_alias.lpp`, `list_safety.lpp` | Pass |
| Alias only inside one branch | `arc_nested_branch_alias.lpp` | Pass |
| Closure capture under branch flow | `arc_closure_branch_capture.lpp` | Pass |
| Float list rejection | `aot_reject_float_list.lpp` | Pass: no object emitted |
| Strong ARC cycle rejection | `aot_reject_arc_cycle.lpp` | Pass: no object emitted |

## 3. Issues found during the audit

### Fixed: Cranelift process-entry ABI

The AOT backend previously exported the L++ `Void` `main` directly as the process entry point. On Linux, this left the process exit status in an undefined register value. Programs that printed a non-zero value could therefore have a non-zero process exit status even when they ran correctly.

**Fix:** the AOT backend now emits:

```text
lpp_main  → internal source-level function
main      → C ABI wrapper returning I32 zero
```

The wrapper calls `lpp_main()` then returns process status `0`.

This repaired parity failures in closure/list/ARC tests where stdout matched but the shell exit status did not.

### Fixed: field-read ownership classification

A custom object loaded from an environment or struct field was temporarily classified as owned, causing a closure body to release an object owned by its environment.

**Fix:** custom/list field reads now produce a borrowed MIR value. A retain occurs only if that borrow is assigned to a new owner or returned as `ReturnOwned`.

## 4. Test results

```text
cargo test
11 passed; 0 failed
```

```text
AOT/C parity suite
20 passed; 0 failed
```

The parity suite compares stdout and process exit status for the supported subset, then verifies both required AOT rejections.

## 5. Retain/release audit result

For the supported subset, ownership edges now follow these rules:

```text
fresh allocation                → one owner, no retain
move temporary                  → transfer, no retain
borrow → second owner           → one retain
borrowed parameter → return     → one retain + ReturnOwned
struct field ownership edge     → one retain
closure environment edge        → one retain
scope exit                      → one release per live owner
ReturnOwned                     → excluded from callee release
final release                   → destructor chain
```

The compiler intentionally rejects strong cyclic struct ownership because ARC alone cannot match those retains with reclamation.

## 6. C backend comparison

The C and Cranelift backends now match **observable output and process status** across the current 18 successful programs.

However, this is not yet proof of ownership parity:

- Cranelift owns the authoritative ARC/MIR contract.
- The C backend still emits directly from the AST rather than ownership MIR.
- C remains a compatibility/debug reference, not the backend that defines safety semantics.

## 7. List[T] ARC milestone outcome

`List[Custom]` is now enabled with the required core ownership contract:

```text
[x] list element retain callback
[x] list element destructor callback
[x] typed get returns a borrow, not an owner
[x] typed push accepts a borrow and creates one list-owned reference
[x] list alias/move/return regression coverage
[x] cycle rule for List[T] ↔ struct ownership graph
[x] C compatibility runtime support
```

`List[Float]`, `List[String]`, closures as list elements, and other element types remain rejected. They need their own value or ownership representation before being safely enabled.
