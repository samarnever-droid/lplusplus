# Language Guide

## Syntax

L++ uses indentation-based blocks and Python-readable definitions:

```lpp
def add(left: Int, right: Int) -> Int:
    return left + right

def main():
    value := add(20, 22)
    print(value)
```

`:=` introduces an inferred binding. Supported scalar types include `Int`, `Float`, `Bool`, and `Str`. Structs and supported generic lists participate in the ownership analysis.

## Control flow

```lpp
if value > 0:
    print("positive")
else:
    print("not positive")

while value > 0:
    value := value - 1
```

## Built-ins

Core built-ins include console I/O, files, JSON compatibility helpers, lists, and TCP networking. The exact current contract belongs in [[Networking]] and the repository `Doc.md`; do not assume undocumented APIs exist.

## AOT boundary

Cranelift AOT is the authoritative implementation for ownership semantics. The C backend remains useful for compatibility/debugging, but it is not the definition of L++ safety.

## v0.1.3 current-status note

This page is maintained with the project, but current support claims are
centralized in [Current Capabilities](../documentation/CURRENT_CAPABILITIES.md).

```text
Use LppData/build/release and LppData/cache for package artifacts.
Use host-linked AOT for filesystem/networking work.
Do not assume direct ELF supports files, networking, JSON, or threads.
Do not claim language-wide Rust-equivalent safety outside the verified AOT subset.
```
