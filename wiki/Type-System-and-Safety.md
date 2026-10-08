# Type System & Safety

L++ features a strong, statically checked type system designed to prevent bugs before code ever reaches runtime. There are no unchecked `null` references, no implicit type coercion, and all type errors are diagnosed at compile time.

---

## 1. Type Inference & Annotations

L++ employs **bidirectional type inference** (Hindley-Milner inspired):
- Local variable bindings infer their concrete type from initialization expressions.
- Function parameter types and public interfaces require explicit annotations to preserve clear architectural boundaries and fast module-level checking.

```lpp
# Types inferred cleanly:
count := 42                # Int
rate := 0.05               # Float
active := true             # Bool
names := ["Alice", "Bob"]  # List[Str]

# Explicit interface typing:
def compute_interest(principal: Float, rate: Float, periods: Int) -> Float:
    return principal * (1.0 + rate) ** float(periods)
```

---

## 2. No Null Pointers: The `Option[T]` Pattern

In L++, primitive values and references cannot be `null`. Absence of a value is explicitly modeled through `Option[T]`:

```lpp
enum Option[T]:
    Some(T)
    None

def find_user(id: Int) -> Option[Str]:
    if id == 1:
        return Option::Some("Alice")
    return Option::None

def main():
    result := find_user(1)
    match result:
        Option::Some(name) =>
            println("Found user: " + name)
        Option::None =>
            println("User not found.")
```

Because `Option[T]` requires pattern matching or unwrapping, "null pointer exceptions" are mathematically impossible at runtime.

---

## 3. Generics & Monomorphization

Generics allow writing reusable algorithms without sacrificing runtime performance:

```lpp
struct Stack[T]:
    items: List[T]

    def push(self, item: T):
        self.items.append(item)

    def pop(self, default_val: T) -> T:
        if self.items.len() > 0:
            return self.items.pop()
        return default_val
```

### Zero-Cost Abstraction
Generics in L++ are fully **monomorphized**:
- During compilation, the compiler generates a specialized, concrete version of each struct and function for every distinct type argument (e.g. `Stack[Int]`, `Stack[Str]`).
- There is zero pointer indirection, zero dynamic boxing, and zero runtime performance penalty.

---

## 4. Traits & Dispatch

Traits define shared interfaces that types can implement:

```lpp
trait Printable:
    def to_string(self) -> Str

struct Point:
    x: Int
    y: Int

impl Printable for Point:
    def to_string(self) -> Str:
        return "(" + str(self.x) + ", " + str(self.y) + ")"

def display[T: Printable](item: T):
    println("Item: " + item.to_string())
```

### Static Dispatch
By default, trait calls on generic parameters use **static monomorphization**. The compiler directly inlines or branches to the concrete implementation without vtable lookups.

---

## 5. Exhaustive Pattern Matching

When matching on algebraic enums, the compiler validates that every possible variant is handled:

```lpp
enum Direction:
    North
    South
    East
    West

def move(d: Direction):
    match d:
        Direction::North => println("Going north")
        Direction::South => println("Going south")
        Direction::East  => println("Going east")
        Direction::West  => println("Going west")
```

If a variant is omitted (e.g., forgetting `West`), the compiler halts compilation with diagnostic `error[E0008]: non-exhaustive pattern match`.

---

## 6. Safety Invariants Summary

| Potential Issue | C / C++ | Python / JS | L++ Guarantee |
|---|---|---|---|
| **Null pointer dereference** | Crash (Segfault) | `NoneType` / `undefined` Exception | **Impossible** (No null; `Option[T]` enforced) |
| **Type mismatch at runtime** | Undefined Behavior | Runtime `TypeError` | **Impossible** (Static type check) |
| **Use-after-free** | Undefined Behavior / Exploit | N/A (GC) | **Prevented** (ARC + borrow validation) |
| **Data race on shared memory** | Undefined Behavior | GIL / Race conditions | **Checked** (Thread isolation & balance) |
| **Uncaught exceptions** | Uncaught crash | Uncaught crash | **Explicit** (`Result[T, E]` return types) |
