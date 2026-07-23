# Error Handling

L++ provides Rust-style error handling through enums, match, and the `?` operator.

## Defining Errors

```lpp
enum Result:
    Ok(value: Int)
    Err(code: Int)
```

Variants with data carry a payload. `Ok(42)` stores the success value, `Err(1)` stores an error code.

## Creating Results

```lpp
def divide(a: Int, b: Int) -> Result:
    if b == 0:
        return Result.Err(1)     # error: division by zero
    return Result.Ok(a / b)      # success with value
```

## Handling Errors with Match

```lpp
r := divide(10, 3)
match r:
    Ok(value):
        print(value)             # prints 3
    Err(code):
        print_str("error!")
        print(code)              # prints error code
```

Match bindings (`value`, `code`) extract the data from the variant.

## Propagating Errors with `?`

The `?` operator unwraps `Ok` or returns `Err` early:

```lpp
def process(x: Int) -> Result:
    a := step1(x)?          # if Err, return it immediately
    b := step2(a)?          # same
    return Result.Ok(b)     # only reached if both succeeded

def main():
    match process(5):
        Ok(v):
            print(v)
        Err(c):
            print_str("pipeline failed")
```

Without `?`, you'd need:
```lpp
# Without ? (verbose)
r := step1(x)
match r:
    Ok(a):
        r2 := step2(a)
        match r2:
            Ok(b):
                return Result.Ok(b)
            Err(c):
                return Result.Err(c)
    Err(c):
        return Result.Err(c)
```

## Option Type

```lpp
enum Option:
    Some(value: Int)
    None

def find_item(lst: List, target: Int) -> Option:
    # ... search logic
    return Option.None          # not found

match find_item(data, 42):
    Some(idx):
        print(idx)
    None:
        print_str("not found")
```

## Helper Functions (stdlib/result.lpp)

```lpp
from stdlib.result import is_ok, unwrap, unwrap_or

r := Result.Ok(42)
print(is_ok(r))         # true
print(unwrap(r))        # 42
print(unwrap_or(r, 0))  # 42 (or 0 if Err)
```

## Internal Representation

Enums are packed into a single `i64`:
- Upper 32 bits: variant tag (0 = first variant, 1 = second, etc.)
- Lower 32 bits: data payload

This means zero allocation, zero ARC overhead for enum values. The `?` operator compiles to a simple integer divide + branch.
