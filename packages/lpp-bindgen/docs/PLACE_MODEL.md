# c2lpp typed C place model

A C place is an assignable scalar or bitfield subobject. `CPlace` flattens the
underlying pointer provenance rather than owning a nested managed `CPtr`:

```text
memory context identity
allocation identity
absolute byte offset
subobject lower/upper bounds
generation
mutability
element/storage width
signedness
optional bit offset/width
```

Flattening is a safety requirement. The first nested-`CPtr` implementation
passed functional tests but leaked one managed subobject under LeakSanitizer.
The replacement performs raw checked access through allocation descriptors and
passes ASan/UBSan/LeakSanitizer.

## Constructors

- scalar dereference;
- relative byte place;
- checked fixed-array index;
- aggregate field;
- nested aggregate field;
- address-of and dereference identity.

## Operations

- signed/unsigned loads and stores for 8/16/32/64-bit storage;
- integer bitfield extraction and neighboring-bit-preserving writes;
- assignment;
- arithmetic and bitwise compound assignment;
- shifts with range checks;
- prefix/postfix increment/decrement;
- swap;
- size-checked overlap-safe copy;
- place identity comparison.

## Explicit failures

```text
C2-PLACE-INDEX-OUT-OF-BOUNDS
C2-PLACE-DIVIDE-BY-ZERO
C2-PLACE-MODULO-BY-ZERO
C2-PLACE-COPY-SIZE
C2-PLACE-SHIFT-RANGE
C2-PLACE-USE-AFTER-FREE
C2-PLACE-STALE-GENERATION
C2-MEM-WRITE-READONLY
```

## Validation

The differential fixture performs 24 ordered operations over a nested C struct
containing an integer array, bitfield and signed field. Native C and generated
L++ output must match byte-for-byte. Four negative binaries verify bounds,
readonly, divide-by-zero and copy-size traps. The successful fixture is also
linked with ASan/UBSan and LeakSanitizer enabled.

## Pointer-valued slots

`CPointerPlace` stores a complete CPtr provenance tuple in a 64-byte internal
compatibility slot. It supports null assignment, checked load/store, arrays of
pointer slots, copy, swap and stale-target validation. This remains useful for
non-ABI compatibility storage.

`CAbiPointerPlace` instead reserves exactly the target's eight-byte field slot.
The bytes contain a null or nonzero marker; complete provenance is keyed by owner
allocation and byte offset in `CMemory`'s raw side table. Thus generated aggregate
layout is not widened to 64 bytes.

## General parser integration

The typed sweep carries scalar pointee kind, explicit ABI width, signedness and
pointer depth in symbol metadata. It lowers direct dereference, checked indexing,
address-of, assignment/compound assignment, postfix updates, local pointers,
arithmetic, difference and equality through `CPtr`/`CPlace`.

A demand-bounded aggregate catalog adds SysV x86-64 places for direct arrow
access, nested by-value chains, integer bitfields and fixed arrays. ABI-width
data-pointer fields use `CAbiPointerPlace`: the aggregate retains an eight-byte
slot while a raw per-context side table stores full provenance. Null, chained
load, assignment, copy, nonoverlap `memcpy` and `realloc` preservation are tested;
stale, untracked, invalidated and pointer-bearing `memmove` cases trap. Native
fixtures and successful binaries pass ASan/UBSan/LeakSanitizer. Pinned SQLite
selects nine parameter-driven pointee types plus one sizeof demand, producing six
complete and three safe-prefix layouts with 100 fields.

## Pointer indirection

A pointer-depth-two value addresses consecutive eight-byte ABI pointer slots.
`*pp` and `pp[index]` load a provenance-bearing CPtr through the side table;
assignments update that slot. One-dimensional array parameters decay to ordinary
CPtr metadata.

## Remaining work

The gate covers integer scalar, bitfield, aggregate, data-pointer and depth-two
pointer places. Floating-point storage, volatile/atomic access, packed/unaligned
ABI policies, pointer depth above two, multidimensional arrays, function-pointer
fields and layouts beyond the explicit demand budget remain separate obligations.
