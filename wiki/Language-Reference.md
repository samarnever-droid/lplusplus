# Language Reference

## Keywords (21)

`as` `break` `continue` `def` `else` `enum` `false` `fn` `for` `from` `if` `import` `in` `match` `mut` `pub` `return` `spawn` `struct` `true` `while`

## Types

| Type | Description | Example |
|------|-------------|---------|
| `Int` | 64-bit signed integer | `42`, `0`, `-1` (via `0 - 1`) |
| `Float` | 64-bit IEEE 754 | `3.14`, `0.0` |
| `Str` | ARC-managed string | `"hello"` |
| `Bool` | Boolean | `true`, `false` |
| `Void` | No return value | (default for functions) |
| `List[T]` | Dynamic list | `list_new()` |
| `Map[K,V]` | Hash map | `map_new()` |
| Custom | Struct/Enum type | `Point`, `Color` |

## Variables

```lpp
x := 42           # immutable, type inferred
mut y := 0        # mutable
y = 10            # reassign (requires mut)
name := "Alice"   # Str
```

## Functions

```lpp
def add(a: Int, b: Int) -> Int:
    return a + b

def greet(name: Str):       # returns Void
    print_str(name)
```

## Structs

```lpp
struct Point:
    x: Int
    y: Int

p := Point(10, 20)
print(p.x)           # 10
```

## Enums

```lpp
enum Color:
    Red
    Green
    Blue

enum Result:
    Ok(value: Int)
    Err(code: Int)

c := Color.Red
r := Result.Ok(42)
e := Result.Err(1)
```

## Match

```lpp
match value:
    Ok(v):
        print(v)
    Err(code):
        print(code)
    Red:
        print_str("red")
```

## Error Propagation

```lpp
def process(x: Int) -> Int:
    v := might_fail(x)?     # ? unwraps Ok or returns Err
    return Result.Ok(v + 1)
```

## Control Flow

```lpp
# If/else
if x > 0:
    print_str("positive")
else:
    print_str("non-positive")

# While
mut i := 0
while i < 10:
    print(i)
    i = i + 1

# For range
for i in range(10):
    print(i)

# For list
for item in lst:
    print(item)

# Break/continue
while true:
    if done:
        break
```

## Closures

```lpp
adder := fn(x: Int) -> Int:
    return x + 10

result := adder(5)    # 15
```

## Threads

```lpp
spawn fn():
    print_str("in thread")
```

## Operators

| Operator | Description |
|----------|-------------|
| `+` `-` `*` `/` `%` | Arithmetic |
| `==` `!=` `<` `>` `<=` `>=` | Comparison |
| `:=` | Declare (immutable) |
| `=` | Assign (requires `mut`) |
| `?` | Try/unwrap Result |
| `.` | Field access / enum variant |
