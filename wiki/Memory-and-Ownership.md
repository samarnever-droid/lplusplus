# Memory & Ownership

L++ provides complete memory safety **without a tracing garbage collector** and without runtime stop-the-world pauses. Memory management is automated, deterministic, and verified at compile time through an ownership analysis pipeline combining static escape analysis, value classification, and deterministic Automatic Reference Counting (ARC).

---

## 1. Core Philosophy: Why No Tracing GC?

Tracing garbage collectors (used in Go, Java, C#, Python) introduce:
1. **Unpredictable Latency:** Garbage collection pauses ("stop-the-world") interrupt real-time systems, games, audio processing, and high-throughput servers.
2. **High Memory Overhead:** Tracing collectors typically require 2x–3x the resident memory to avoid thrashing CPU caches with frequent collection sweeps.
3. **Nondeterministic Destructors:** Operating system resources (file handles, network sockets, GPU textures) are closed arbitrarily late when the collector happens to run.

L++ adopts **deterministic, scope-bound destruction**: as soon as a value's last owner goes out of scope, its memory and associated resources are released immediately on the current thread.

---

## 2. Value Classification: Frame, Owned, and Shared

During the `lpp-ownership` pass, the compiler categorizes every managed variable into one of three classifications:

| Category | Storage | Lifecycle | Overhead |
|---|---|---|---|
| **`Frame`** | Call Stack | Bound strictly to current function frame | Zero allocation cost; pointer increment/decrement |
| **`Owned`** | Native Heap | Unique single owner; freed immediately on scope exit | Standard allocation, zero reference count tracking |
| **`Shared`** | Native Heap | Multiple owners across scopes or threads; managed by ARC | Atomic increment on clone, atomic decrement on drop |

```
                Allocation Request
                        │
            Can value escape function?
                   ╱          ╲
                 No            Yes
                 │              │
             [Frame]      Multiple owners?
           Stack Allocated     ╱          ╲
                             No            Yes
                             │              │
                         [Owned]        [Shared]
                        Single Heap    Atomic Ref Counted
```

---

## 3. Escape Analysis & Stack Promotion

Before allocating a struct or buffer on the heap, the compiler runs an **escape analysis pass**:

```lpp
struct Vector3:
    x: Float
    y: Float
    z: Float

def calculate_magnitude(x: Float, y: Float, z: Float) -> Float:
    v := Vector3(x, y, z)   # Classified as Frame!
    return (v.x * v.x + v.y * v.y + v.z * v.z) ** 0.5
```

Because `v` never escapes `calculate_magnitude`:
1. It is promoted to stack allocation (`Frame`).
2. No heap `malloc` or `free` calls are generated.
3. Accesses compile down to direct stack offset loads and stores.

---

## 4. Automatic ARC Synthesis

When a value genuinely escapes (e.g. stored in a returned list, captured by a closure, or passed between threads), the compiler synthesizes ARC balance instructions:

```lpp
def create_greeting(name: Str) -> Str:
    # 'name' is retained when entering, and 'msg' is transferred to the caller
    msg := "Hello, " + name
    return msg
```

### Static Balance Checks
The compiler verifies that every branch through a function has balanced `retain` and `release` counts:
- Variables exiting scope without escaping have an automatic `release` instruction inserted at the exit block.
- Early `return` statements, loop `break`s, and exception paths automatically trigger cleanup blocks for all live variables in the scope.
- Double-frees and memory leaks are ruled out by construction.

---

## 5. Compile-Time Borrow Verification

L++ allows creating borrowed references (`&T`) and borrowed string slices (`StrSlice`) without incurring reference counting operations:

```lpp
def print_length(slice: StrSlice):
    # 'slice' is a borrowed view: 0 allocations, 0 reference counts
    println("Length: " + str(slice.len()))

def main():
    text := "Heavy heap-allocated string"
    view := text[0:5] # Zero-copy stack slice
    print_length(view)
```

The borrow verifier enforces:
1. **No Outlived Borrows:** A borrow cannot be returned from a function if it references local stack memory.
2. **Borrow Invalidation:** Modifying a collection while active borrows exist is rejected at compile time.

---

## 6. Comparison with Other Languages

| Feature | L++ | Rust | Go | C++ | Python |
|---|---|---|---|---|---|
| **Memory Model** | ARC + Escape Analysis | Strict Borrow Checker | Tracing GC | Manual / Smart Ptrs | Tracing GC + RefCount |
| **GC Pauses** | **0 ms** | 0 ms | 1–10+ ms | 0 ms | Unpredictable |
| **Ergonomics** | High (automatic) | Strict (explicit lifetimes) | High | Low (manual safety) | High |
| **Destructor Timing** | **Immediate (RAII)** | Immediate (RAII) | Nondeterministic | Immediate (RAII) | Nondeterministic |
| **Memory Footprint** | Minimal | Minimal | 2x–3x overhead | Minimal | High |
