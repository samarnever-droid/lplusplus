# Verified Examples

These examples were checked in a clean temporary documentation project with:

```bash
target/release/lpp --checkall
```

The validation run passed:

```text
[L++] --checkall: OK — 12 file(s) passed
```

## Hello world

```lpp
def main():
    print_str("Hello from L++!")
    print(42)
```

## Variables and control flow

```lpp
const LIMIT = 10

def main():
    mut total := 0
    for i in range(0, LIMIT, 2):
        total += i
    if total > 10 && total < 100:
        print(total)
    else:
        print(0)
```

## Functions with defaults

```lpp
def add(a: Int, b: Int = 10) -> Int:
    return a + b

def power(base: Int, exp: Int = 2) -> Int:
    mut result := 1
    for i in range(exp):
        result *= base
    return result

def main():
    print(add(5))
    print(add(5, 20))
    print(power(3))
    print(power(2, 10))
```

## Struct method syntax

```lpp
struct Point:
    x: Int
    y: Int

def magnitude_squared(p: Point) -> Int:
    return p.x * p.x + p.y * p.y

def main():
    p := Point(3, 4)
    print(p.magnitude_squared())
```

## Errors and `?`

```lpp
enum Result:
    Ok(value: Int)
    Err(code: Int)

def safe_divide(a: Int, b: Int) -> Result:
    if b == 0:
        return Result.Err(1)
    return Result.Ok(a / b)

def compute(a: Int, b: Int) -> Result:
    value := safe_divide(a, b)?
    return Result.Ok(value + 100)
```

## Imports

```lpp
import utils.mathx
from stdlib.math import pow

def main():
    print(double(21))
    print(triple(10))
    print(pow(2, 8))
```
