# Type System and Safety

L++ is designed to be safe without a tracing garbage collector.

## Immutable by default

```lpp
x := 1
mut y := 2
y += 3
```

A non-`mut` binding cannot be reassigned or have fields mutated.

## Parameters are immutable

Function parameters cannot be reassigned directly. Use a local mutable copy:

```lpp
def double_it(x: Int) -> Int:
    mut result := x
    result *= 2
    return result
```

## Ownership model

L++ uses a hybrid strategy:

| Value kind | Strategy |
|---|---|
| primitives | copied |
| strings | ARC-managed |
| structs | stack unless escaping |
| escaping structs | ARC heap |
| containers | ARC handles |
| ownership cycles | rejected or arena-classified |

## Escape analysis

The compiler checks whether a value escapes its local scope.

Examples of escaping:

- returned from a function
- stored in a list or map
- captured by a closure

Escaping values are promoted to ARC-managed storage.

## Cycle rejection

Owned cycles are rejected because ARC alone cannot reclaim them.

```lpp
struct Node:
    next: Node   # rejected as cyclic owned struct
```

This prevents a large class of memory leaks.

## Generics safety

Generics are phase 1. Type parameters are accepted by the type checker and erased to the native value representation (`i64`). Return type inference works at call sites — `identity(42)` correctly infers the result is `Int`. Full trait bounds and monomorphization are future work.

## Traits and static dispatch

Traits define method interfaces. `impl` blocks provide implementations for specific types.

```lpp
trait Area:
    def area(self) -> Int

impl Area for Circle:
    def area(self) -> Int:
        return 3 * self.radius * self.radius
```

L++ supports both **static** and **dynamic** dispatch:

- **Static dispatch**: when the receiver's concrete type is known at compile time, the compiler resolves `c.area()` → `Circle_area(c)` directly via name mangling. Zero runtime cost.
- **Dynamic dispatch**: when a function accepts a trait-typed parameter (`def f(x: Speak)`), the compiler passes hidden function pointers at the call site. Inside the function, method calls dispatch through those pointers via `CallIndirect`. This is similar to Go's interfaces or Rust's `impl Trait`.

## Current limitations

- Type aliases are parsed but full substitution is still experimental.
- Generic enum payloads and rich `Result[T]` style APIs are on the roadmap.
- Trait bounds on generic parameters (`T: Display`) are not yet supported.
- Trait conformance is not enforced (missing methods not flagged at `impl` time).
- Some older scratch examples in the repository intentionally fail safety checks.
