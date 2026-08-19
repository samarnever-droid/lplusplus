# L++ (LPlusPlus) Project Instructions & Style Rules

Refer to `AGENTS.md` at the project root for the complete language syntax, standard library, and package management reference.

## Key Rules
1. **Always use exact L++ types**: `Int`, `Float`, `Str`, `Bool`, `Char`, `Void`, `List[T]`, `Map[K, V]`, `(T1, T2)`.
2. **Variable binding**: Use `:=` for declaring new variables with inferred types, and `=` for reassigning existing variables.
3. **Strings**: Strongly typed `Str`. Use `print_str(s)` to print strings and `print(n)` to print integers.
4. **Entry Point**: Always define `def main():` in executable roots.
5. **Memory Safety**: Memory is automatically managed by compile-time deterministic ARC + static cyclebreaking. No manual memory freeing required.
6. **GUI / Desktop Apps**: Use native `webview_window_create`, `webview_navigate`, `webview_run`, `webview_destroy`.
