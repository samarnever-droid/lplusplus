# S1 Rejection Contract

S1 means an unsupported or unsafe ownership/linking request is rejected deterministically before a usable native executable is emitted. S1 is not proof that all accepted programs are safe.

## Mandatory rejection classes

| Class | Required result |
|---|---|
| Strong owned struct cycle | AOT diagnostic: ARC cannot reclaim ownership cycles; no object output |
| Strong owned list/aggregate cycle | Same cycle diagnostic; no object output |
| Unsupported AOT element representation | Diagnostic: not supported safely yet; no object output |
| Malformed native object | Direct linker rejects input; no executable output |
| Unresolved runtime import | Direct linker rejects input; no executable output |
| Unsupported direct-link platform/mode | Clear error; never emit an executable known to violate platform policy |

## S1 rule

Every new language/runtime/linker feature must add its unsafe or unsupported case to a named negative test before adding documentation that advertises the happy path. Error text is part of the contract where tooling depends on it; executable non-production is the stronger invariant.
