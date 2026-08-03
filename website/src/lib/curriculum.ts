export interface QuizOption {
  text: string;
  isCorrect: boolean;
}

export interface LessonChallenge {
  id: string;
  title: string;
  type: "quiz" | "code";
  prompt: string;
  initialCode?: string;
  expectedOutput?: string;
  options?: QuizOption[];
  solutionCode?: string;
  explanation: string;
}

export interface LessonTheory {
  title: string;
  summary: string;
  explanationMarkdown: string;
  codeExample: string;
  keyTakeaways: string[];
}

export interface Lesson {
  id: string;
  title: string;
  description: string;
  xpReward: number;
  theory: LessonTheory;
  challenges: LessonChallenge[];
}

export interface Stage {
  id: string;
  title: string;
  level: "Beginner" | "Intermediate" | "Advanced" | "Master";
  icon: string;
  description: string;
  lessons: Lesson[];
}

export const CURRICULUM: Stage[] = [
  {
    id: "stage-1",
    title: "1. L++ Foundations",
    level: "Beginner",
    icon: "🌱",
    description: "Learn L++ syntax, explicit typing, functions, and immutable vs mutable bindings.",
    lessons: [
      {
        id: "lesson-1-1",
        title: "Hello L++ & Functions",
        description: "Understand `def` function definitions and `print_str` console output.",
        xpReward: 20,
        theory: {
          title: "What is `def` and `print_str` in L++?",
          summary: "In L++, code is organized into explicit typed functions, starting with `def main() -> Void:`.",
          explanationMarkdown: `Welcome to L++! L++ is a fast, native language designed to feel as clean as Python while running as fast as C/Rust.

### 1. Defining Functions with \`def\`
Every function in L++ starts with the **\`def\`** keyword.
- \`def main() -> Void:\` defines the main entry point function.
- \`-> Void\` specifies that this function does not return any value.
- The colon **\`:\`** at the end opens an indented block.

### 2. Printing Text with \`print_str\`
L++ provides built-in console logging functions:
- **\`print_str("Hello World")\`**: Prints raw string text to standard output.
- **\`print(123)\`**: Prints numeric values or formatted data to standard output.

Let's look at a complete working L++ program:`,
          codeExample: `def main() -> Void:
    # Print a greeting to standard output
    print_str("Hello L++!")
    print(42)`,
          keyTakeaways: [
            "Use `def` to define functions in L++",
            "`def main() -> Void:` is the entry point of your program",
            "`print_str` outputs string text to the screen",
            "`print` outputs integers and general values"
          ]
        },
        challenges: [
          {
            id: "c-1-1-1",
            title: "Function Definition Keyword",
            type: "quiz",
            prompt: "Which keyword is used in L++ to define a function?",
            options: [
              { text: "def", isCorrect: true },
              { text: "function", isCorrect: false },
              { text: "fn", isCorrect: false },
              { text: "func", isCorrect: false }
            ],
            explanation: "In L++, functions are defined using the explicit `def` keyword, just like `def main() -> Void:`!"
          },
          {
            id: "c-1-1-2",
            title: "Print String Output",
            type: "code",
            prompt: "Complete the function to output 'Hello L++!' using `print_str`.",
            initialCode: "def main() -> Void:\n    # Write your print_str statement below\n    print_str(\"Hello L++!\")",
            solutionCode: "def main() -> Void:\n    print_str(\"Hello L++!\")",
            expectedOutput: "Hello L++!",
            explanation: "`print_str(\"Hello L++!\")` outputs raw text strings to standard output."
          }
        ]
      },
      {
        id: "lesson-1-2",
        title: "Variables & Mutability (`mut` vs `:=`) font",
        description: "Master immutable bindings (`:=`) and mutable variables (`mut`).",
        xpReward: 25,
        theory: {
          title: "Immutable by Default: How `:=` and `mut` Work",
          summary: "L++ variables are 100% immutable by default to prevent accidental bugs. Use `mut` when values must change.",
          explanationMarkdown: `In traditional languages, variables can change at any time, causing subtle state bugs. L++ introduces safety by default!

### 1. Immutable Bindings (\`:=\`)
When you declare a variable like \`x := 10\`, L++ infers its type as \`Int\` and locks its value. You cannot reassign \`x = 20\` later!

### 2. Mutable Variables (\`mut\`)
If you need a variable to change (like a counter or loop index), declare it with **\`mut\`**:
- \`mut count := 10\`
- \`count = count + 5\`  # Allowed because count is marked mutable!

### 3. Printing Integers
Use **\`print(value)\`** to print numeric variables to the screen.`,
          codeExample: `def main() -> Void:
    # Immutable variable (cannot change)
    name := "Alice"
    
    # Mutable variable (can change)
    mut score := 100
    score = score + 50
    
    print_str(name)
    print(score)`,
          keyTakeaways: [
            "`x := value` creates an immutable variable that cannot be reassigned",
            "`mut x := value` creates a mutable variable that can be changed with `=`",
            "Immutability prevents race conditions and accidental data corruption"
          ]
        },
        challenges: [
          {
            id: "c-1-2-1",
            title: "Default Binding Rule",
            type: "quiz",
            prompt: "By default, a variable declared with `count := 10` is:",
            options: [
              { text: "Immutable (cannot be reassigned)", isCorrect: true },
              { text: "Mutable (can be reassigned anytime)", isCorrect: false },
              { text: "Global variable", isCorrect: false }
            ],
            explanation: "L++ variables declared with `:=` are immutable by default to ensure memory safety!"
          },
          {
            id: "c-1-2-2",
            title: "Build a Mutable Score Tracker",
            type: "code",
            prompt: "Declare a mutable variable `mut score := 10`, add 5 to it, and print it with `print(score)`.",
            initialCode: "def main() -> Void:\n    mut score := 10\n    score = score + 5\n    print(score)",
            solutionCode: "def main() -> Void:\n    mut score := 10\n    score = score + 5\n    print(score)",
            expectedOutput: "15",
            explanation: "`mut score := 10` declares a mutable integer. `score = score + 5` updates it to 15."
          }
        ]
      }
    ]
  },
  {
    id: "stage-2",
    title: "2. Control Flow & Structs",
    level: "Intermediate",
    icon: "⚡",
    description: "Branching with if/else, while loops, and value-type structs.",
    lessons: [
      {
        id: "lesson-2-1",
        title: "If, Elif, Else Branching",
        description: "Control program flow with boolean conditions.",
        xpReward: 30,
        theory: {
          title: "Conditionals in L++",
          summary: "Use `if`, `elif`, and `else` with Pythonic colon `:` syntax for clean control flow.",
          explanationMarkdown: `Conditionals allow your program to make decisions based on runtime conditions.

### Syntax Overview
- **\`if condition:\`**: Evaluates a boolean expression.
- **\`elif condition:\`**: Evaluates an alternate condition if the previous was false.
- **\`else:\`**: Executes when no previous condition matched.

No parentheses are required around conditions!`,
          codeExample: `def main() -> Void:
    age := 20
    if age >= 21:
        print_str("Full Adult")
    elif age >= 18:
        print_str("Adult")
    else:
        print_str("Minor")`,
          keyTakeaways: [
            "Conditions end with a colon `:`",
            "Indentation defines block scope",
            "Use `==`, `!=`, `<`, `>`, `<=`, `>=` for comparisons"
          ]
        },
        challenges: [
          {
            id: "c-2-1-1",
            title: "Check Age Eligibility",
            type: "code",
            prompt: "Write an if-statement checking if `age >= 18` and print 'Adult'.",
            initialCode: "def main() -> Void:\n    age := 20\n    if age >= 18:\n        print_str(\"Adult\")",
            solutionCode: "def main() -> Void:\n    age := 20\n    if age >= 18:\n        print_str(\"Adult\")",
            expectedOutput: "Adult",
            explanation: "`if age >= 18:` evaluates true and executes `print_str(\"Adult\")`."
          }
        ]
      },
      {
        id: "lesson-2-2",
        title: "Struct Value Types",
        description: "Zero-overhead stack-allocated data structures.",
        xpReward: 35,
        theory: {
          title: "Structs: Stack-Allocated Data Types",
          summary: "Define composite data structures with explicit field types stored directly on the Stack.",
          explanationMarkdown: `Structs group related data together. Unlike Java or Python objects which require heavy heap allocation and garbage collection, L++ structs are **Stack-allocated by default** with zero overhead!

### Defining a Struct
Use the **\`struct\`** keyword followed by typed fields:
\`\`\`
struct Vector2D:
    x: Int
    y: Int
\`\`\`

### Instantiating Structs
Construct instances using positional arguments:
\`v := Vector2D(10, 20)\``,
          codeExample: `struct Point:
    x: Int
    y: Int

def main() -> Void:
    p := Point(10, 20)
    print(p.x + p.y)`,
          keyTakeaways: [
            "`struct Name:` defines a custom value type",
            "Struct fields have explicit types (`x: Int`)",
            "Stack allocation ensures zero garbage collection overhead!"
          ]
        },
        challenges: [
          {
            id: "c-2-2-1",
            title: "Create and Sum Point Struct",
            type: "code",
            prompt: "Instantiate `Point(10, 20)` and print the sum of `p.x + p.y`.",
            initialCode: "struct Point:\n    x: Int\n    y: Int\n\ndef main() -> Void:\n    p := Point(10, 20)\n    print(p.x + p.y)",
            solutionCode: "struct Point:\n    x: Int\n    y: Int\n\ndef main() -> Void:\n    p := Point(10, 20)\n    print(p.x + p.y)",
            expectedOutput: "30",
            explanation: "`Point(10, 20)` creates a stack struct. `p.x + p.y` equals `30`."
          }
        ]
      }
    ]
  },
  {
    id: "stage-3",
    title: "3. Safe Systems Memory & CPtr",
    level: "Advanced",
    icon: "🛡️",
    description: "Master CPtr fat pointers, generation tracking, and memory sanitizers without unsafe code.",
    lessons: [
      {
        id: "lesson-3-1",
        title: "Safe Checked C Pointers (`CPtr`)",
        description: "Safe checked C pointer allocations via stdlib/c_memory.",
        xpReward: 45,
        theory: {
          title: "Safe C Memory: CPtr & Provenance",
          summary: "L++ provides 100% safe C pointer manipulation without `unsafe` blocks using `CPtr` fat pointers.",
          explanationMarkdown: `In standard C or C++, pointers can easily cause segfaults, out-of-bounds reads, and Use-After-Free vulnerabilities. L++ eliminates this completely with **\`CPtr\`** fat pointers in \`stdlib/c_memory.lpp\`!

### What is a CPtr?
A **\`CPtr\`** is a 64-bit fat pointer carrying provenance metadata:
- **Base Pointer**: Heap address
- **Offset**: Current byte offset
- **Size Bounds**: Subobject memory limits
- **Generation ID**: Prevents Use-After-Free (UAF)

Accessing memory outside \`CPtr\` bounds raises a clean L++ diagnostic panic instead of an OS crash!`,
          codeExample: `import c_memory

def main() -> Void:
    # Create memory heap context
    mem := c_memory_new(16)
    
    # Allocate checked fat pointer CPtr
    ptr := c_malloc(mem, 32)
    
    c_store_u32(ptr, 1337)
    print(c_load_u32(ptr))
    
    c_free(ptr)
    c_memory_destroy(mem)`,
          keyTakeaways: [
            "`import c_memory` provides safe checked C pointer operations",
            "`CPtr` tracks allocation bounds and generation IDs",
            "Out-of-bounds or use-after-free operations trigger safe catchable diagnostics"
          ]
        },
        challenges: [
          {
            id: "c-3-1-1",
            title: "Out of Bounds Safety",
            type: "quiz",
            prompt: "What happens if a CPtr attempts to read beyond its allocated size in L++?",
            options: [
              { text: "L++ raises a safe diagnostic panic with provenance info", isCorrect: true },
              { text: "Operating System segfault crash", isCorrect: false },
              { text: "Silent memory corruption", isCorrect: false }
            ],
            explanation: "CPtr tracks allocation bounds, turning illegal memory access into 100% safe catchable diagnostics!"
          },
          {
            id: "c-3-1-2",
            title: "Allocate & Store Memory",
            type: "code",
            prompt: "Allocate 32 bytes, store u32 integer 999, and print it with `c_load_u32`.",
            initialCode: "import c_memory\n\ndef main() -> Void:\n    mem := c_memory_new(16)\n    ptr := c_malloc(mem, 32)\n    c_store_u32(ptr, 999)\n    print(c_load_u32(ptr))\n    c_free(ptr)\n    c_memory_destroy(mem)",
            solutionCode: "import c_memory\n\ndef main() -> Void:\n    mem := c_memory_new(16)\n    ptr := c_malloc(mem, 32)\n    c_store_u32(ptr, 999)\n    print(c_load_u32(ptr))\n    c_free(ptr)\n    c_memory_destroy(mem)",
            expectedOutput: "999",
            explanation: "`c_malloc` allocates checked `CPtr` memory. `c_store_u32` and `c_load_u32` write and read integers safely."
          }
        ]
      }
    ]
  }
];
