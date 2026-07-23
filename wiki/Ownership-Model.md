# Ownership Model

L++ uses a hybrid memory model with automatic ownership management.

## Three Storage Classes

| Class | When | How |
|-------|------|-----|
| **Stack** | Scalars (`Int`, `Float`, `Bool`), non-escaping structs | Zero-cost, no allocation |
| **Heap (ARC)** | Structs that escape (returned, stored in containers) | Atomic reference counting |
| **Arena** | Self-referential structs | Region-based allocation |

## Escape Analysis

The compiler automatically determines storage class:

```lpp
def example():
    x := 42           # Stack (scalar, doesn't escape)
    p := Point(1, 2)  # Stack if not returned/stored
    return p           # Promoted to Heap (escapes via return)
```

### Rules
1. **Return escape**: Values returned from functions → Heap
2. **Container escape**: Values stored in Lists/Maps → Heap
3. **Cycle detection**: Self-referential structs → Arena (or rejected)

## ARC Operations

The MIR pass automatically inserts:
- `Retain` — increment reference count (when value is shared)
- `Release` — decrement reference count (when scope ends)
- `ReturnOwned` — transfer ownership out of function

```lpp
def identity(item: Item) -> Item:
    return item
    # MIR: item is Borrowed → Retained → ReturnOwned
```

## Cycle Rejection

L++ statically detects ownership cycles and rejects them:

```lpp
struct Node:
    next: Node    # ERROR: Cyclic owned struct detected
```

This guarantees ARC can always reclaim memory — no leaks, no tracing GC.

## What Users See

Nothing. The ownership model is invisible:

```lpp
struct Box:
    value: Int

def create() -> Box:
    return Box(42)       # automatically heap-allocated

def main():
    b := create()        # ownership transferred
    print(b.value)       # borrowed access
    # b automatically released at scope exit
```

No `Arc`, `Rc`, `Box`, `&`, `*`, `unsafe`, or manual free.
