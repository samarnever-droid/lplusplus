# L++ Language Tour

L++ is designed to combine the clean syntax and rapid ergonomics of high-level scripting languages with the zero-cost abstractions, strict types, and predictability of native systems programming.

---

## 1. Syntax Basics & Indentation

L++ uses **significant whitespace** for block structure (like Python), eliminating braces (`{}`) while keeping code clean and readable.

```lpp
def main():
    x := 10
    if x > 5:
        println("x is greater than 5")
    else:
        println("x is 5 or less")
```

Comments begin with `#` and run to the end of the line:
```lpp
# This is a single-line comment
x := 42 # inline comment
```

---

## 2. Variables & Assignment

Variables are declared and initialized using `:=`. Type annotations can be provided explicitly or inferred:

```lpp
# Inferred types
count := 10             # Int
ratio := 3.14           # Float
active := true          # Bool
message := "Hello L++"  # Str

# Explicit type annotations
total: Int = 100
pi: Float = 3.14159
title: Str = "Systems Programming"

# Re-assignment uses '='
total = 105
```

---

## 3. Primitive Types

| Type | Description | Example |
|---|---|---|
| `Int` | 64-bit signed integer | `42`, `-10` |
| `Float` | 64-bit IEEE 754 floating-point | `3.14`, `-0.5` |
| `Bool` | Boolean truth value | `true`, `false` |
| `Str` | UTF-8 dynamic string managed by ARC | `"hello world"` |
| `Void` | Unit / no return value | Implicit function return |

---

## 4. Control Flow

### If / Elif / Else
```lpp
score := 85

if score >= 90:
    println("Grade: A")
elif score >= 80:
    println("Grade: B")
elif score >= 70:
    println("Grade: C")
else:
    println("Grade: F")
```

### While Loops
```lpp
i := 0
while i < 5:
    println(str(i))
    i = i + 1
```

### For Loops
Iterating over lists or ranges:
```lpp
items := [10, 20, 30, 40]
for item in items:
    println(str(item))
```

Loops support `break` to exit early and `continue` to advance to the next iteration.

---

## 5. Functions

Functions are defined with `def`. Parameter types and return types are strongly enforced.

```lpp
def add(a: Int, b: Int) -> Int:
    return a + b

def greet(name: Str):
    println("Hello, " + name)
```

### Default Parameters
Parameters can specify default values:
```lpp
def connect(host: Str, port: Int = 8080, timeout_ms: Int = 5000):
    println("Connecting to " + host + ":" + str(port))
```

---

## 6. Structs

Structs define custom composite data structures:

```lpp
struct Point:
    x: Float
    y: Float

def distance_from_origin(p: Point) -> Float:
    return (p.x * p.x + p.y * p.y) ** 0.5

def main():
    pt := Point(3.0, 4.0)
    d := distance_from_origin(pt)
    println(str(d)) # 5.0
```

Structs can also define methods:
```lpp
struct Counter:
    value: Int

    def increment(self):
        self.value = self.value + 1

    def get(self) -> Int:
        return self.value
```

---

## 7. Enums & Pattern Matching

Enums represent algebraic data types with optional associated data:

```lpp
enum Status:
    Pending
    InProgress(Int) # percentage
    Completed(Str)  # result string
    Failed(Str)     # error message

def log_status(s: Status):
    match s:
        Status::Pending =>
            println("Waiting to start...")
        Status::InProgress(pct) =>
            println("Progress: " + str(pct) + "%")
        Status::Completed(msg) =>
            println("Success: " + msg)
        Status::Failed(err) =>
            println("Error: " + err)
```

The compiler checks pattern matching exhaustiveness statically.

---

## 8. Collections

### Lists
Lists are dynamically sized, heap-allocated, ARC-managed homogeneous sequences:
```lpp
numbers := [1, 2, 3, 4, 5]
numbers.append(6)
println(str(numbers.len())) # 6
println(str(numbers[0]))     # 1
```

### Tuples
Tuples are fixed-size structural heterogeneous groupings:
```lpp
pair: (Str, Int) = ("Alice", 25)
name := pair.0
age := pair.1
```

### Maps
Key-value mappings:
```lpp
scores := Map[Str, Int]()
scores.insert("Alice", 98)
scores.insert("Bob", 85)

if scores.contains("Alice"):
    println("Alice scored: " + str(scores.get("Alice")))
```

---

## 9. Closures & Anonymous Functions

Functions are first-class values. Closures can capture variables from their enclosing scope:

```lpp
def make_multiplier(factor: Int) -> fn(Int) -> Int:
    return |x: Int| -> Int:
        return x * factor

def main():
    double := make_multiplier(2)
    triple := make_multiplier(3)

    println(str(double(5))) # 10
    println(str(triple(5))) # 15
```

---

## 10. Error Handling: Result & `?`

L++ does not use unchecked exceptions. Fallible functions return `Result[T, E]`:

```lpp
enum Result[T, E]:
    Ok(T)
    Err(E)

def parse_int(s: Str) -> Result[Int, Str]:
    if s == "42":
        return Result::Ok(42)
    return Result::Err("Invalid number")

def compute() -> Result[Int, Str]:
    # The '?' operator unpacks Ok or propagates Err immediately
    val := parse_int("42")?
    return Result::Ok(val * 2)
```

---

## 11. Generics

Functions, structs, and enums can be parameterized with type variables:

```lpp
def identity[T](val: T) -> T:
    return val

struct Box[T]:
    item: T

def main():
    b_int := Box[Int](10)
    b_str := Box[Str]("boxed text")
    println(b_str.item)
```

Generics are fully monomorphized at compile time, guaranteeing zero runtime overhead.
