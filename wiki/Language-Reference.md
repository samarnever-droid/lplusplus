# Language Reference

This is the practical syntax reference for L++.

## Comments

```lpp
# Single-line comment
```

L++ currently uses `#` single-line comments. Block comments are not part of the language yet.

## Literals

```lpp
42              # decimal Int
1_000_000       # Int with separators
0xFF            # hexadecimal Int
0b1010          # binary Int
3.14            # Float
true            # Bool
false           # Bool
"hello"         # Str
"""
multiline
string
"""             # multiline Str
f"hello {name}" # f-string interpolation for string expressions
[1, 2, 3]       # list literal
```

## Core types

| Type | Meaning |
|---|---|
| `Int` | 64-bit signed integer |
| `Float` | 64-bit floating point |
| `Str` | ARC-managed string |
| `Bool` | boolean |
| `Void` | no value |
| `List[Int]` | dynamic list handle |
| `Map[Int, Int]` | map handle |
| custom structs | user-defined records |
| custom enums | tagged values |
| type params | `T`, `A`, `B` in generics |

## Variables

```lpp
x := 42
mut total := 0
total += x
```

Variables are immutable by default. Use `mut` if the variable will be reassigned.

## Constants

Constants are top-level declarations:

```lpp
const LIMIT = 10

def main():
    print(LIMIT)
```

## Functions

```lpp
def add(a: Int, b: Int) -> Int:
    return a + b

def greet(name: Str):
    print_str(name)
```

Functions returning nothing omit the return type.

## Default parameters

```lpp
def add(a: Int, b: Int = 10) -> Int:
    return a + b

def main():
    print(add(5))       # 15
    print(add(5, 20))   # 25
```

## Operators

| Category | Operators |
|---|---|
| Arithmetic | `+`, `-`, `*`, `/`, `%` |
| Augmented assignment | `+=`, `-=`, `*=`, `/=`, `%=` |
| Comparison | `==`, `!=`, `<`, `>`, `<=`, `>=` |
| Logical | `&&`, `||`, `!` |
| Bitwise | `&`, `|`, `^`, `<<`, `>>` |
| Unary | `-x`, `!flag` |
| Try | `?` |
| Access | `.`, `[]` |
| Declare / assign | `:=`, `=` |

`&&` and `||` short-circuit.

## Control flow

```lpp
const LIMIT = 10

def main():
    mut total := 0
    for i in range(0, LIMIT, 2):
        total += i

    if total > 10 && total < 100:
        print(total)
    elif total == 10:
        print(10)
    else:
        print(0)
```

Supported loops:

```lpp
for i in range(10):
    print(i)

for i in range(2, 10):
    print(i)

for i in range(0, 10, 2):
    print(i)

while condition:
    # body
    break
```

## Structs

```lpp
struct Point:
    x: Int
    y: Int

p := Point(3, 4)
print(p.x)
```

## Method syntax / UFCS

L++ supports method-call syntax as sugar for free functions.

```lpp
struct Point:
    x: Int
    y: Int

def magnitude_squared(p: Point) -> Int:
    return p.x * p.x + p.y * p.y

def main():
    p := Point(3, 4)
    print(p.magnitude_squared())  # calls magnitude_squared(p)
```

## Enums and match

```lpp
enum Result:
    Ok(value: Int)
    Err(code: Int)

def safe_divide(a: Int, b: Int) -> Result:
    if b == 0:
        return Result.Err(1)
    return Result.Ok(a / b)

def main():
    match safe_divide(10, 2):
        Ok(v):
            print(v)
        Err(code):
            print(code)
```

Enum values are currently represented internally as a packed `i64` tag/data pair. This works for integer payloads and forms the basis of Result-style error handling.

## Generics, phase 1

```lpp
def identity[T](x: T) -> T:
    return x

struct Box[T]:
    value: T

def main():
    print(identity(42))
    s := identity("generic string")
    print_str(s)

    b := Box(99)
    print(b.value)
```

Current limitations:

- Generic function inference works for common call-site cases.
- Generic values are erased to `i64` in codegen.
- Generic functions cannot yet dispatch overloaded builtins such as `print(x)` when `x: T`; use concrete calls like `print_str` where needed.
- Trait bounds and monomorphization are future work.

## Closures

```lpp
def main():
    base := 100
    add_base := fn(x: Int) -> Int:
        return x + base

    print(add_base(5))
```

## Threads

```lpp
def main():
    spawn fn():
        print_str("running in background")

    print_str("main continues")
```

## Strings

```lpp
def main():
    name := "world"
    msg := f"hello {name}!"
    print_str(msg)

    print_str(char_at("abc", 1))
    print(ord("A"))
    print_str(chr(65))
    print(str_find("hello world", "world"))
```

## Lists and maps

```lpp
def main():
    nums := [10, 20, 30]
    print(list_len(nums))
    print(nums[0])

    m := map_new()
    map_put(m, 1, 100)
    print(map_get(m, 1))
```
