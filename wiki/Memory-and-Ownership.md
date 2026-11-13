# Memory & Ownership

L++ manages memory **without a stop-the-world tracing garbage collector** on its native runtime, combining MIR escape/placement classification, balanced Automatic Reference Counting (ARC), ownership balance proofs (`E4403`), and Bacon-Rajan trial-deletion cycle collection on Cranelift native targets.

> For verified safety scope and backend boundaries, see [`documentation/Safety_Mission.md`](../documentation/Safety_Mission.md), [`documentation/CYCLE_BREAKING.md`](../documentation/CYCLE_BREAKING.md), and [`documentation/CONCURRENCY.md`](../documentation/CONCURRENCY.md).

---

## 1. Three-Tier Ownership Classification (`Frame`, `Owned`, `Shared`)

During `lpp-ownership` analysis over MIR, every managed value is placed into one of three tiers:

| Tier | Storage | Lifecycle | Notes |
|---|---|---|---|
| **`Frame`** | Call Stack | Scoped to current function frame | Non-escaping ordinary structs/values; avoids heap allocation |
| **`Owned`** | Heap (ARC) | Single owning flow | Move semantics avoid redundant retain/release pairs along linear moves |
| **`Shared`** | Heap (ARC) | Aliased or potentially cycle-capable | Participates in cycle tracking on Cranelift native targets |

*(In the current native runtime, heap ARC headers use atomic reference count operations across managed cells; see [`documentation/CONCURRENCY.md`](../documentation/CONCURRENCY.md).)*

---

## 2. Native ARC Headers vs WASM Layout

- **Native Cranelift / Hosted Runtime**:
  - Destructor-free managed objects (including `Str`) use a **16-byte common header** immediately before payload (`magic`, atomic `refcount`, `generation`, `flags`).
  - Destructor-bearing native objects prepend an **8-byte destructor pointer** before that common header (**24-byte total allocation prefix**), keeping payload at `common_header + 16`.
- **WebAssembly (`wasm32-wasip1`)**:
  - Uses its own target-specific 24-byte header (`[refcount i64][drop-table index i64][magic]`) with size-class free-list reuse for acyclic churn.

---

## 3. Escape Analysis & Stack Promotion

```lpp
struct Vector3:
    x: Float
    y: Float
    z: Float

def length_sq(x: Float, y: Float, z: Float) -> Float:
    v := Vector3(x, y, z)   # Non-escaping -> eligible for Frame placement
    return v.x * v.x + v.y * v.y + v.z * v.z
```

Because `v` does not escape `length_sq`, it does not need heap retention beyond the frame.

---

## 4. Ownership Balance Proofs & Cycle Collection

### Compile-Time Balance Proof (`E4403`)
Before codegen, `lpp-ownership` verifies balanced retain/release paths along the MIR CFG (catching use-after-move or missing ownership cells with `E4403`), and MIR tests run the ARC interpreter leak oracle (`E4304`).

### Runtime Trial-Deletion Collector (Cranelift Native)
- `lpp-ownership` identifies types whose containment edges can form a cycle via strongly connected components (SCCs) and classifies them as `Shared` (collectible).
- On Cranelift native targets, `lpp-runtime` reclaims unreachable cycles among `Shared` objects using a Bacon-Rajan trial-deletion collector (`lpp_arc_cycle_collect`). Acyclic objects continue to drop immediately when their reference count hits zero.

### Current Backend & Concurrency Boundaries
- **WASM (`wasm32-wasip1`)**: Reuses released acyclic heap blocks, but does **not** run cycle collection yet; cyclic WASM graphs can leak.
- **LLVM (`--backend llvm`)**: Managed structs/lists/tuples and cycle-collector parity remain incomplete in `v0.2.0`.
- **Threads (`spawn`)**: Refcounts are atomic, but compile-time `Send`/`Sync` enforcement and race-freedom proofs are not yet implemented (`specs.md` §6.3).

---

## 5. Borrowed Slices (`StrSlice` and `Slice[T]`)

Zero-copy borrowed views avoid heap copies when passed downward into known readers:

```lpp
def show_slice(s: Str):
    view := str_slice(s, 0, 5)
    print_str(str_slice_to_str(view))
```

In the current first tier, conservative escape checks reject returning borrowed slices, capturing them across escaping closures/`spawn`, or mutating their source while borrowed.
