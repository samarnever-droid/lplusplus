# L++ Language Tour

L++ combines clean, indentation-based syntax with static type checking, type inference, and native execution.

> Authoritative grammar & operator reference: [`specs.md`](../specs.md) §2–§5.

---

## 1. Syntax Basics & Indentation

L++ uses **significant whitespace** for block structure, introduced by `:` and indented with spaces (tabs are rejected):

```lpp
def main():
    x := 10
    if x > 5:
        print_str("x is greater than 5")
    else:
        print_str("x is 5 or less")
```

Both `#` and `//` start a single-line comment.

---

## 2. Variables & Mutability

Bindings are immutable by default with `:=`; use `mut` when reassignment is needed:

```lpp
# Immutable inferred bindings
count := 10             # Int
ratio := 3.14           # Float
active := true          # Bool
message := "Hello L++"  # Str

# Explicit type annotation
total: Int = 100

# Mutable binding & reassignment
mut score := 10
score = 15
score += 5
```

---

## 3. Primitive & Core Types

| Type | Description | Example |
|---|---|---|
| `Int` | 64-bit signed integer | `42`, `-10` |
| `Float` | 64-bit IEEE-754 float | `3.14`, `-0.5` |
| `Bool` | Boolean value | `true`, `false` |
| `Char` | Unicode scalar character literal | `'A'` |
| `Str` | NUL-terminated UTF-8 ARC-managed string | `"hello world"` |
| `Void` | No return value | Default return type |
| `List[T]` | Managed dynamic list | `[1, 2, 3]` |
| `(A, B)` | Fixed structural tuple (arity 2–4) | `(10, "ok")` |

Fixed-width integer types (`U8`/`I8`, `U16`/`I16`, `U32`/`I32` and lowercase aliases) are also supported, primarily for `extern "C"` signatures.

---

## 4. Control Flow

### `if` / `elif` / `else`
```lpp
score := 85

if score >= 90:
    print_str("Grade: A")
elif score >= 80:
    print_str("Grade: B")
else:
    print_str("Grade: C")
```

### `while` and `for` Loops
```lpp
mut i := 0
while i < 5:
    print(i)
    i = i + 1

for n in 0..5:
    print(n)

items := [10, 20, 30]
for item in items:
    print(item)
```

Loops support `break` and `continue`.

---

## 5. Functions, Defaults, Variadics, and Closures

```lpp
def add(a: Int, b: Int) -> Int:
    return a + b

def connect(host: Str, port: Int = 8080):
    print_str(f"Connecting to {host} on port " + int_to_str(port))

def sum_all(...numbers: Int) -> Int:
    mut total := 0
    for n in numbers:
        total = total + n
    return total
```

Closures use `fn(...) -> ...`:
```lpp
double := fn(x: Int) -> Int: x * 2
```

---

## 6. Structs, Methods, Enums, and Traits

```lpp
struct Point:
    x: Float
    y: Float = 0.0

impl Point:
    def dist_sq(self) -> Float:
        return self.x * self.x + self.y * self.y

enum Direction:
    North
    South
    Move(x: Int, y: Int)

def step(d: Direction):
    match d:
        Direction.North =>
            print_str("North")
        Direction.South =>
            print_str("South")
        Direction.Move(x, y) =>
            print_str("Move")

trait Display:
    def to_str(self) -> Str

def show[T: Display](item: T):
    print_str(item.to_str())
```

---

## 7. Modules & Error Propagation (`?`)

Modules map dotted paths (`import math.vector` -> `math/vector.lpp`). Functions returning `Result` / `Option` values can propagate early errors with `?`. See [`specs.md`](../specs.md) §3.4 & §5.1.
