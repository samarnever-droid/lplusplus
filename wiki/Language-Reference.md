# Language Reference

## Comments

```lpp
# This is a single-line comment
# L++ uses # for comments (no block comments)
```

## Literals

```lpp
42                # Int (64-bit signed)
3.14              # Float (64-bit IEEE 754)
true              # Bool
false             # Bool
"hello world"     # Str (ARC-managed string)
```

## Keywords (21)

`as` `break` `continue` `def` `else` `enum` `false` `fn` `for` `from` `if` `import` `in` `match` `mut` `pub` `return` `spawn` `struct` `true` `while`

## Types

| Type | Description | Example |
|------|-------------|---------|
| `Int` | 64-bit signed integer | `42` |
| `Float` | 64-bit IEEE 754 | `3.14` |
| `Str` | ARC-managed string | `"hello"` |
| `Bool` | Boolean | `true`, `false` |
| `Void` | No return value | default for functions |
| `List[Int]` | Dynamic list | `list_new()` |
| `Map[Int, Int]` | Hash map | `map_new()` |
| Custom | Struct or Enum type | `Point`, `Result` |

## Variables

```lpp
x := 42              # immutable, type inferred from value
mut y := 0           # mutable — can be reassigned
y = 10               # reassign (only works with mut)
name := "Alice"      # Str
pi := 3.14           # Float
flag := true         # Bool
```

Immutable by default. Use `mut` to allow reassignment.

## Functions

```lpp
def add(a: Int, b: Int) -> Int:
    return a + b

def greet(name: Str):          # returns Void (implicit)
    print_str(name)
```

Parameters are always immutable inside the function body. L++ does not support `mut` parameters — if you need to modify a value, declare a local mutable copy:

```lpp
def double_it(x: Int) -> Int:
    mut result := x         # local mutable copy
    result = result * 2
    return result
```

## Structs

```lpp
struct Point:
    x: Int
    y: Int

p := Point(10, 20)      # positional constructor
print(p.x)              # field access → 10
```

## Enums

Unit variants (no data):

```lpp
enum Color:
    Red
    Green
    Blue

c := Color.Green
```

Data-carrying variants:

```lpp
enum Result:
    Ok(value: Int)
    Err(code: Int)

enum Token:
    Number(value: Int)
    Identifier(name: Int)
    EOF
```

Constructing:

```lpp
r := Result.Ok(42)
e := Result.Err(1)
t := Token.EOF
```

## Pattern Matching

```lpp
match token:
    Number(v):
        print(v)
    Identifier(id):
        print(id)
    EOF:
        print_str("end of file")
```

Match extracts data from variants via bindings (`v`, `id`).

```lpp
def describe(r: Result):
    match r:
        Ok(value):
            print_str("success:")
            print(value)
        Err(code):
            print_str("error code:")
            print(code)
```

## Error Handling

Define fallible functions with `Result` return type:

```lpp
enum Result:
    Ok(value: Int)
    Err(code: Int)

def safe_divide(a: Int, b: Int) -> Result:
    if b == 0:
        return Result.Err(1)
    return Result.Ok(a / b)
```

Handle errors with `match`:

```lpp
match safe_divide(10, 3):
    Ok(v):
        print(v)
    Err(c):
        print_str("division failed")
```

Propagate errors with `?`:

```lpp
def process(a: Int, b: Int) -> Result:
    v := safe_divide(a, b)?      # returns Err immediately if failed
    return Result.Ok(v + 100)    # only reached on success
```

## Control Flow

```lpp
# If / else
if x > 0:
    print_str("positive")
else:
    print_str("non-positive")

# While loop
mut i := 0
while i < 10:
    print(i)
    i = i + 1

# For range
for i in range(10):
    print(i)            # 0 through 9

# For-in (list iteration)
for item in my_list:
    print(item)

# Break and continue
mut n := 0
while true:
    n = n + 1
    if n == 5:
        continue        # skip 5
    if n > 10:
        break           # stop at 10
    print(n)
```

## Closures

```lpp
adder := fn(x: Int) -> Int:
    return x + 10

print(adder(5))    # 15
```

Closures capture variables from the enclosing scope:

```lpp
base := 100

add_base := fn(x: Int) -> Int:
    return x + base      # captures 'base'

print(add_base(5))    # 105
```

## Threads

```lpp
spawn fn():
    print_str("running in background")

print_str("main continues immediately")
```

## Imports

```lpp
import math                          # loads math.lpp
import utils.helpers                 # loads utils/helpers.lpp
from stdlib.math import abs, pow     # selective import
from stdlib.result import is_ok      # import specific functions
```

## Operators

| Category | Operators |
|----------|-----------|
| Arithmetic | `+` `-` `*` `/` `%` |
| Comparison | `==` `!=` `<` `>` `<=` `>=` |
| Declaration | `:=` (declare immutable) |
| Assignment | `=` (reassign, requires `mut`) |
| Try | `?` (unwrap Result or return Err) |
| Access | `.` (field / enum variant) |

L++ does not currently support bitwise operators (`&`, `|`, `^`, `<<`, `>>`), logical operators (`&&`, `||`), or unary operators (`!`, `-x`). Use comparisons and arithmetic instead:

```lpp
# Instead of !flag:
if flag == false:

# Instead of -x:
neg := 0 - x

# Instead of a && b:
if a:
    if b:
        # both true
```

## Ownership

- **Primitives** (`Int`, `Float`, `Bool`) are copied — no ownership tracking.
- **Str** is ARC-managed — reference counted, automatically freed.
- **Structs** are stack-allocated by default. Promoted to ARC heap when they escape (returned from functions, stored in containers).
- **List** and **Map** are ARC-managed.
- **Ownership cycles are rejected at compile time** — no tracing GC needed.

Values move by default. The compiler inserts retain/release automatically:

```lpp
def create() -> Point:
    return Point(1, 2)    # ownership transferred to caller

def main():
    p := create()         # p owns the Point
    print(p.x)            # borrowed access
    # p is automatically released when main() exits
```

## Standard Library Quick Reference

```lpp
# Console
print(42)                          # print Int
print_str("hello")                 # print Str
input()                            # read line → Str

# Strings
str_len("abc")                     # → 3
str_concat("a", "b")              # → "ab"
str_repeat("ha", 3)               # → "hahaha"

# Lists
mut lst := list_new()
list_push(lst, 10)
list_get(lst, 0)                   # → 10
list_len(lst)                      # → 1

# Maps
m := map_new()
map_put(m, 1, 100)
map_get(m, 1)                     # → 100

# File I/O
write_file("out.txt", "data")
content := read_file("out.txt")

# Range
for i in range(5):                # 0, 1, 2, 3, 4
    print(i)
```

## Generics

Functions, structs, and enums can be parameterized with type variables using square-bracket syntax.

### Generic Functions

```lpp
def identity[T](x: T) -> T:
    return x

def first[A, B](a: A, b: B) -> A:
    return a

# Usage — type is inferred from arguments:
x := identity(42)        # T = Int → returns Int
s := identity("hello")   # T = Str → returns Str
```

### Generic Structs

```lpp
struct Box[T]:
    value: T

b := Box(42)
print(b.value)   # 42
```

### Generic Enums

```lpp
enum Option[T]:
    Some(value: T)
    None

enum Result[T, E]:
    Ok(value: T)
    Err(error: E)
```

> **Note:** Generic type parameters are erased to `i64` at the Cranelift backend level (type erasure). This means all values are uniformly represented as 64-bit words. Full monomorphization is planned for a future release.

## Default Parameter Values

Function parameters can have default values. When calling, omitted trailing arguments use their defaults.

```lpp
def add(a: Int, b: Int = 10) -> Int:
    return a + b

def greet(name: Str, greeting: Str = "Hello"):
    print(str_concat(greeting, str_concat(" ", name)))

# Usage:
add(5)        # → 15 (b defaults to 10)
add(5, 20)    # → 25 (b overridden)
greet("world")         # → Hello world
greet("world", "Hi")   # → Hi world
```

## String Operations

```lpp
# Character access
char_at("hello", 0)       # → "h"
ord("A")                  # → 65
chr(65)                   # → "A"

# Search
str_find("hello", "ll")   # → 2 (index, or -1)
str_contains("hello", "ll")  # → 1 (true)

# Prefix/suffix
str_starts_with("hello", "hel")  # → 1
str_ends_with("hello", "lo")    # → 1

# Case
str_upper("hello")        # → "HELLO"
str_lower("HELLO")        # → "hello"

# Whitespace
str_trim("  hello  ")     # → "hello"

# Replace
str_replace("foo bar foo", "foo", "baz")  # → "baz bar baz"

# Conversion
int_to_str(42)            # → "42"
str_to_int("123")         # → 123

# Existing ops
str_len("hello")          # → 5
str_concat("a", "b")     # → "ab"
str_substr("hello", 1, 3) # → "ell"
str_repeat("ha", 3)      # → "hahaha"
```
