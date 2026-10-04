# OpenCode TUI Architecture — How the Interactive UI Works

This page documents how [OpenCode](https://github.com/anomalyco/opencode) builds its terminal UI, analyzed from source for porting to L++.

## Stack Overview

```
User terminal (iTerm2, Windows Terminal, etc.)
    │
    ├── @opentui/core         ← Low-level terminal renderer (ANSI escape codes)
    ├── @opentui/solid         ← Solid.js reactive bindings for terminal components
    ├── @opentui/keymap        ← Keyboard shortcut system (Vim-like modes)
    │
    └── packages/tui/          ← OpenCode TUI app
        ├── app.tsx            ← Entry point, creates renderer at 60fps
        ├── routes/            ← Pages (Home, Session)
        ├── component/         ← UI components (Prompt, Logo, Dialogs)
        ├── context/           ← Shared state (theme, SDK, sync, clipboard)
        └── ui/                ← Reusable widgets (Toast, Dialog, Border)
```

## How @opentui/core Renders to Terminal

The core rendering works through an `OptimizedBuffer` — a 2D grid of cells, each with a character, foreground color, background color, and text attributes. On each frame:

1. **Solid.js reactivity** detects state changes (user input, API responses)
2. **Component tree** re-renders affected JSX nodes
3. **Layout engine** measures and positions components in the terminal grid
4. **OptimizedBuffer** diffs the new frame against the previous frame
5. **Only changed cells** are flushed to stdout via ANSI escape sequences

### Key ANSI sequences used:

```
\x1b[H          — Move cursor to top-left
\x1b[2J         — Clear entire screen
\x1b[{row};{col}H  — Move cursor to position
\x1b[38;5;{n}m  — Set foreground color (256-color)
\x1b[48;5;{n}m  — Set background color (256-color)
\x1b[0m         — Reset all attributes
\x1b[1m         — Bold
\x1b[2m         — Dim
\x1b[?25l       — Hide cursor
\x1b[?25h       — Show cursor
\x1b[?1049h     — Enter alternate screen buffer
\x1b[?1049l     — Leave alternate screen buffer
\x1b[?997;1n    — Dark theme detection query
```

### createCliRenderer options:

```typescript
createCliRenderer({
    externalOutputMode: "passthrough",
    targetFps: 60,           // 60 frames per second
    gatherStats: false,
    exitOnCtrlC: false,      // Custom Ctrl+C handling
    useKittyKeyboard: {},    // Advanced keyboard protocol
    autoFocus: false,
    openConsoleOnError: false,
})
```

## How the Prompt Component Works

`packages/tui/src/component/prompt/index.tsx`

The prompt is a `TextareaRenderable` (from @opentui/core) that handles:

- Multi-line text editing with cursor movement
- Paste events (decoded from terminal paste brackets)
- Autocomplete suggestions (fuzzy search via `fuzzysort`)
- File attachment drag-and-drop
- Prompt history (up/down arrow)
- Vim-like keybindings via `@opentui/keymap`
- Shell mode vs normal mode

### Input handling flow:

```
Raw stdin bytes → Terminal keyboard protocol → KeyEvent/MouseEvent/PasteEvent
    → Keymap resolver (checks mode: normal, insert, visual)
    → Command dispatch (submit, cancel, navigate, edit)
    → State update (Solid.js signal/store)
    → Re-render affected components
```

## What L++ Needs to Port This

### Minimum viable TUI in L++ (no framework):

```lpp
# 1. Raw terminal mode
extern "C":
    def tcgetattr(fd: Int, termios: Int) -> Int
    def tcsetattr(fd: Int, opt: Int, termios: Int) -> Int
    def read(fd: Int, buf: Int, count: Int) -> Int

# 2. ANSI escape helpers (already in stdlib/color.lpp)
def cursor_to(row: Int, col: Int) -> Str:
    return str_concat("\033[", str_concat(int_to_str(row), \
        str_concat(";", str_concat(int_to_str(col), "H"))))

def clear_screen() -> Str:
    return "\033[2J\033[H"

def hide_cursor() -> Str:
    return "\033[?25l"

def show_cursor() -> Str:
    return "\033[?25h"

# 3. Screen buffer (2D grid)
# Each cell: character + color
# On render: diff against previous buffer, emit only changes

# 4. Event loop
# while true:
#     read stdin (non-blocking)
#     dispatch key events
#     update state
#     render changed cells
```

### Layers needed:

| Layer | OpenCode uses | L++ equivalent |
|---|---|---|
| Terminal raw mode | Node.js `process.stdin.setRawMode(true)` | FFI `tcgetattr`/`tcsetattr` |
| ANSI rendering | @opentui/core OptimizedBuffer | Custom buffer in L++ using `print_str` |
| Reactive state | Solid.js signals/stores | Manual state + render-on-change |
| Component layout | @opentui/solid JSX | Function-based layout (`draw_box`, `draw_text`) |
| Keyboard handling | @opentui/keymap | Custom keymap via raw stdin bytes |
| Async I/O | Node.js event loop | Polling with `net_recv` timeout |

### What's already available in L++:

- ✅ ANSI colors (`stdlib/color.lpp` — red, green, blue, bold, etc.)
- ✅ String building (str_concat, int_to_str, char_at, ord, chr)
- ✅ File I/O (read_file, write_file)
- ✅ Networking (net_connect, net_send, net_recv — for API calls)
- ✅ JSON parsing (`stdlib/json.lpp`)
- ✅ HTTP client (`stdlib/http.lpp`)
- ✅ Time/sleep (time_ms, sleep_ms)
- ✅ FFI for raw terminal control (extern "C" for tcgetattr)

### What's missing:

- ❌ Raw terminal mode (need FFI to tcgetattr/tcsetattr)
- ❌ Non-blocking stdin read (need poll/select or read with timeout)
- ❌ Screen buffer diff engine (need to build)
- ❌ Layout engine (need to build)

## Simplified Port Strategy

Instead of porting OpenCode's full React-like TUI framework, port the **core interaction loop**:

```
1. Enter raw terminal mode (FFI)
2. Clear screen, draw initial UI (ANSI)
3. Loop:
   a. Read keystroke (raw stdin)
   b. If Enter → send prompt to LLM API (http_post_raw)
   c. Stream response tokens (net_recv in chunks)
   d. Render each token to screen (print_str with ANSI positioning)
   e. If Ctrl+C → cleanup and exit
4. Restore terminal mode
```

This gives a working coding agent in ~300 lines of L++, using only existing stdlib modules + a small FFI shim for terminal raw mode.
