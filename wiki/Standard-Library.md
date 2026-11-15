# Standard Library & Builtins

L++ provides runtime builtins in `abi/builtins.toml` plus pure-L++ modules under `stdlib/`. Note that backend coverage varies between Cranelift native, WASM (`wasm32-wasip1`), and experimental LLVM (`docs/CURRENT_CAPABILITIES.md`).

---

## 1. Core I/O Functions

```lpp
def main():
    print_str("Hello, world!")     # String + newline
    print(42)                      # Type-directed print + newline
    write_str("Inline: ")          # Without trailing newline
    write_int(7)
    write_nl()

    line := input()                # Reads one line from stdin
    print_str("Read: " + line)
```

---

## 2. String Manipulation

```lpp
s := "Systems Programming"
length := len(s)                   # Byte length (or str_len(s))
greeting := "Hello, " + "L++!"     # Concatenation

n_str := int_to_str(12345)         # "12345"
f_str := float_to_str(3.14)        # Float formatting
b_str := bool_to_str(true)         # "true"
num := str_to_int("42")            # 42
```

### Borrowed String Slices (`StrSlice`)
```lpp
text := "The quick brown fox"
word := str_slice(text, 4, 5)      # Borrowed view (offset 4, len 5)
print_str(str_slice_to_str(word))  # Convert to owned Str when escaping
```

---

## 3. Collections (`List[T]` and `Map[K, V]`)

### Lists
```lpp
mut nums := [10, 20, 30]
list_push(nums, 40)
count := len(nums)
first := nums[0]
nums[1] = 25

for val in nums:
    print(val)
```

### Maps
```lpp
mut inventory := map_new()
map_put(inventory, "Apples", 50)
map_put(inventory, "Oranges", 35)

if map_has(inventory, "Apples"):
    qty := map_get(inventory, "Apples")
    print_str("Apples in stock: " + int_to_str(qty))

print(map_len(inventory))
map_remove(inventory, "Oranges")
```

---

## 4. File I/O

Supported on hosted native builds and over WASI preopens on `wasm32-wasip1`:

```lpp
write_file("config.txt", "key=value\n")
if file_exists("config.txt"):
    content := read_file("config.txt")
    print_str(content)
append_file("config.txt", "extra=1\n")
delete_file("config.txt")
```

---

## 5. Math & Native C FFI (`extern "C"`)

Math helpers include `abs`, `min`, `max`, `floor`, `ceil`, `sqrt`, `pow` / `lpp_pow`, and `int_pow` (L++ does not have a `**` operator).

On Cranelift native targets, `extern "C"` supports direct non-variadic calls for `Bool`, `Int`, `Float`, `U8`/`I8`, `U16`/`I16`, `U32`/`I32`, and borrowed `Str`:
```lpp
link "sqlite3"

extern "C":
    def sqlite3_libversion() -> Str
```
See [`docs/EXTERNAL_C_ABI.md`](../docs/EXTERNAL_C_ABI.md) for ABI preconditions.
