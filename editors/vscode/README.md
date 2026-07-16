# L++ Language Support for Visual Studio Code

This extension provides rich, production-grade language tooling for the **L++** programming language directly in Visual Studio Code.

![L++ Logo](lpp-logo.svg)

---

## Features

### 1. Syntax Highlighting
Full semantic and TextMate token coloring for all L++ syntactical elements:
* **Keywords**: `def`, `struct`, `fn`, `mut`, `spawn`, `return`, `if`, `else`, `while`
* **Primitive Types**: `Int`, `String`, `Bool`, `Void`
* **Operators**: Assignment (`:=`), reassignment (`=`), comparisons (`==`, `!=`, `<`, `>`), and mathematical operators.
* **Built-in Functions**: `print`, `print_str`, `input`, `read_file`, `write_file`

### 2. Live Compiler Diagnostics
Automatically compiles the active document in the background using your local `lpp.exe` binary on file save/change. Heuristically maps parser, semantic, and typecheck errors directly to the corresponding lines in the code editor, highlighting them in real-time.

### 3. Escape Analysis Hover Tips (Compiler-Editor Integration)
Hovering over any local variable query the local L++ escape analyzer to display the variable's **Storage Classification** directly inside the tooltip:
* **`Value`**: Displays stack-allocation optimization details (zero-cost).
* **`Arc`**: Informs you that the compiler has automatically promoted the variable to the Heap under reference counting (due to escaping scopes or thread crossing).
* **`Arena`**: Highlights bump-allocated region storage for graph-like data structures.

### 4. Dynamic Auto-Completion
Provides instant suggestions for:
* L++ Keywords and Type names.
* Built-in standard library function signatures.
* Dynamic symbol completion (re-scans the active document to find defined functions, structs, and variables on the fly!).

### 5. Go to Definition (Ctrl + Click)
Jump directly to the definition of a variable binding, a custom struct layout, or a function block definition anywhere in the document.

### 6. Outline & Breadcrumbs Symbols
Exposes custom `def` functions and `struct` outlines to the VS Code Outline Panel for rapid project navigation.

---

## Installation & Setup

1. Copy or symlink this folder (`editors/vscode`) into your global VS Code extensions directory:
   * **Windows**: `%USERPROFILE%\.vscode\extensions\lpp-vscode`
   * **macOS/Linux**: `~/.vscode/extensions/lpp-vscode`
2. Build the L++ compiler in release profile:
   ```bash
   cargo build --release
   ```
3. Open any `.lpp` file in VS Code. The extension will activate automatically and locate `target/release/lpp.exe` in the workspace root to drive background analyzer hovers and diagnostics!

---

## License
MIT License. Created by the L++ Language Foundation.
