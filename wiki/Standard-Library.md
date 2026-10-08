# Standard Library & Builtins

L++ provides a comprehensive suite of built-in functions, primitive operations, and collection data structures designed for high-performance systems development.

---

## 1. Core I/O Functions

### Printing
```lpp
# Print with automatic newline
println("Hello, world!")

# Print values without trailing newline
print("Processing: ")
print(str(42))

# Specialized fast ASCII / UTF-8 string output
print_str("Direct string output\n")
```

### Reading Input
```lpp
# Read a single line of input from stdin
name := input("Enter your name: ")
println("Welcome, " + name)
```

---

## 2. String Manipulation

Strings in L++ are UTF-8 encoded and managed with deterministic ARC.

### Basic String Operations
```lpp
s := "Systems Programming"

# Length in bytes
length := s.len()               # 19

# Concatenation with '+'
greeting := "Hello, " + "L++!"

# Conversions to Str
n_str := str(12345)             # "12345"
f_str := str(3.14)              # "3.14"
b_str := str(true)              # "true"
```

### Slicing & Substrings
Zero-copy borrowed string slicing (`StrSlice`):
```lpp
text := "The quick brown fox"
word := text[4:9]               # "quick"
prefix := text[0:3]             # "The"
```

---

## 3. Collections

### Lists (`List[T]`)
Dynamically resizable arrays allocated on the heap:

```lpp
# Instantiation
nums := [10, 20, 30]

# Appending items
nums.append(40)

# Length
count := nums.len()             # 4

# Indexing (0-based)
first := nums[0]                # 10
nums[1] = 25                    # update element

# Removing the last element
last := nums.pop()              # 40

# Iteration
for val in nums:
    println(str(val))
```

### Maps (`Map[K, V]`)
Hash maps for efficient key-value lookups:

```lpp
# Instantiation
inventory := Map[Str, Int]()

# Insertion
inventory.insert("Apples", 50)
inventory.insert("Oranges", 35)

# Lookup
if inventory.contains("Apples"):
    qty := inventory.get("Apples")
    println("Apples in stock: " + str(qty))

# Removal
inventory.remove("Oranges")
```

---

## 4. File I/O

Standard synchronous file operations:

```lpp
# Writing text to a file
content := "Configuration data\nkey=value\n"
write_file("config.txt", content)

# Reading text from a file
read_back := read_file("config.txt")
println("File content:\n" + read_back)
```

---

## 5. Math & Conversions

| Function / Operator | Signature | Description |
|---|---|---|
| `abs(x)` | `(Int) -> Int`, `(Float) -> Float` | Absolute value |
| `min(a, b)` | `(Int, Int) -> Int`, `(Float, Float) -> Float` | Minimum of two values |
| `max(a, b)` | `(Int, Int) -> Int`, `(Float, Float) -> Float` | Maximum of two values |
| `a ** b` | Operator | Exponentiation ($a^b$) |
| `int(x)` | `(Float) -> Int`, `(Str) -> Int` | Parse or convert to 64-bit integer |
| `float(x)` | `(Int) -> Float`, `(Str) -> Float` | Convert to 64-bit floating point |

---

## 6. C Foreign Function Interface (FFI)

L++ can call arbitrary C libraries directly using `extern "C"`:

```lpp
extern "C" link "msvcrt":
    def puts(s: Str) -> Int
    def sqrt(x: Float) -> Float

def main():
    puts("Direct C puts invocation!")
    root := sqrt(16.0)
    println("Square root of 16 is: " + str(root))
```

The compiler adheres strictly to the target platform's C calling convention (MSVC x64 ABI on Windows, System V AMD64 ABI on Linux).
