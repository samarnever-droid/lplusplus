# Errors and Result

L++ uses enum-based error handling.

## Define a Result type

```lpp
enum Result:
    Ok(value: Int)
    Err(code: Int)
```

## Return success or failure

```lpp
def safe_divide(a: Int, b: Int) -> Result:
    if b == 0:
        return Result.Err(1)
    return Result.Ok(a / b)
```

## Handle with match

```lpp
match safe_divide(10, 2):
    Ok(v):
        print(v)
    Err(code):
        print(code)
```

## Propagate with `?`

```lpp
def compute(a: Int, b: Int) -> Result:
    value := safe_divide(a, b)?
    return Result.Ok(value + 100)
```

The `?` operator means:

1. Evaluate the expression.
2. If it is `Ok(v)`, continue with `v`.
3. If it is `Err(e)`, immediately return that error from the current function.

## Current limitations

`Result` is not yet generic as `Result[T, E]` in the standard library. You can define concrete enum shapes today, usually with integer payloads. Full generic Result and richer payload support are on the self-hosting roadmap.
