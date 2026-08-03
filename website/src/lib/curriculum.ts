export interface QuizOption {
  text: string;
  isCorrect: boolean;
}

export type StepType = "theory" | "quiz" | "code";

export interface LessonStep {
  id: string;
  type: StepType;
  title: string;
  // For Theory Step
  conceptTitle?: string;
  conceptSummary?: string;
  explanationMarkdown?: string;
  codeExample?: string;
  // For Quiz Step
  prompt?: string;
  options?: QuizOption[];
  // For Code Step
  initialCode?: string;
  solutionCode?: string;
  expectedOutput?: string;
  // Explanation after answer
  explanation?: string;
}

export interface Lesson {
  id: string;
  title: string;
  description: string;
  xpReward: number;
  steps: LessonStep[];
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
    description: "Start with Pythonic simplicity, then master functions and type safety.",
    lessons: [
      {
        id: "lesson-1-1",
        title: "Hello World & Printing",
        description: "Feel right at home with Python-like print statements.",
        xpReward: 20,
        steps: [
          {
            id: "step-1-1-1",
            type: "theory",
            title: "Step 1: Pythonic Simplicity",
            conceptTitle: "Welcome to L++! It feels just like Python.",
            conceptSummary: "If you know Python, you already know L++! You can print text to the screen instantly with `print(...)`.",
            explanationMarkdown: `L++ is designed to look and feel as clean as Python, but compile directly into high-performance native machine code!

To output text or numbers to the console, use **\`print(...)\`**:
\`\`\`
print("Hello World")
print(42)
\`\`\`

No complex setup is required to start printing!`,
            codeExample: `def main() -> Void:
    print("Hello World")`
          },
          {
            id: "step-1-1-2",
            type: "quiz",
            title: "Quick Check",
            prompt: "How do you print text to the screen in L++?",
            options: [
              { text: "print(\"Hello World\")", isCorrect: true },
              { text: "System.out.println(\"Hello World\")", isCorrect: false },
              { text: "console.log(\"Hello World\")", isCorrect: false }
            ],
            explanation: "In L++, `print(\"Hello World\")` works just like Python!"
          },
          {
            id: "step-1-1-3",
            type: "code",
            title: "Practice: Print Your First Message",
            prompt: "Complete the code to print 'Hello World' using `print`.",
            initialCode: "def main() -> Void:\n    # Write your print statement below\n    print(\"Hello World\")",
            solutionCode: "def main() -> Void:\n    print(\"Hello World\")",
            expectedOutput: "Hello World",
            explanation: "`print(\"Hello World\")` outputs raw string text to the screen."
          },
          {
            id: "step-1-1-4",
            type: "theory",
            title: "Step 2: Why `print_str` Exists",
            conceptTitle: "Understanding `print` vs `print_str`",
            conceptSummary: "While `print` is polymorphic, `print_str` is a hyper-optimized native string output built-in.",
            explanationMarkdown: `Why does L++ also have **\`print_str\`** alongside \`print\`?

- **\`print(value)\`**: Polymorphic printer — works for integers, booleans, floats, and strings.
- **\`print_str("text")\`**: Direct native string output — skips type checks and writes string bytes straight to standard output with zero overhead!

In high-performance applications, \`print_str\` is used for ultra-fast text emission.`,
            codeExample: `def main() -> Void:
    # Hyper-fast string printing
    print_str("Zero-overhead string output!")`
          },
          {
            id: "step-1-1-5",
            type: "quiz",
            title: "Quick Check: `print_str`",
            prompt: "Why would an L++ developer use `print_str` instead of `print`?",
            options: [
              { text: "It is a direct native string output with zero type-check overhead", isCorrect: true },
              { text: "It converts numbers into strings automatically", isCorrect: false },
              { text: "It is required for every line of code", isCorrect: false }
            ],
            explanation: "`print_str` bypasses type checking for ultra-fast raw string emission!"
          }
        ]
      },
      {
        id: "lesson-1-2",
        title: "Defining Functions with `def`",
        description: "Learn how L++ defines reusable code blocks.",
        xpReward: 25,
        steps: [
          {
            id: "step-1-2-1",
            type: "theory",
            title: "Step 1: The `def` Keyword",
            conceptTitle: "Creating Reusable Functions",
            conceptSummary: "Just like Python uses `def greet():`, L++ uses `def` to define functions.",
            explanationMarkdown: `Functions allow you to group code into reusable blocks.

In L++, function definitions start with the **\`def\`** keyword:
\`\`\`
def greet():
    print("Hello from L++!")
\`\`\`

Notice the colon **\`:\`** at the end of the \`def\` line! The code inside the function is indented with 4 spaces.`,
            codeExample: `def greet() -> Void:
    print("Hello from a function!")

def main() -> Void:
    greet()`
          },
          {
            id: "step-1-2-2",
            type: "quiz",
            title: "Quick Check: Function Keyword",
            prompt: "Which keyword defines a function in L++?",
            options: [
              { text: "def", isCorrect: true },
              { text: "func", isCorrect: false },
              { text: "function", isCorrect: false },
              { text: "fn", isCorrect: false }
            ],
            explanation: "In L++, functions are defined using `def`, matching Python's clean syntax!"
          },
          {
            id: "step-1-2-3",
            type: "code",
            title: "Practice: Write a Function",
            prompt: "Call `print(\"L++ Rocks!\")` inside the `main` function.",
            initialCode: "def main() -> Void:\n    print(\"L++ Rocks!\")",
            solutionCode: "def main() -> Void:\n    print(\"L++ Rocks!\")",
            expectedOutput: "L++ Rocks!",
            explanation: "`def main() -> Void:` defines the main entry function."
          }
        ]
      },
      {
        id: "lesson-1-3",
        title: "Variables (`:=` vs `mut`)",
        description: "Understand immutability and state changes.",
        xpReward: 30,
        steps: [
          {
            id: "step-1-3-1",
            type: "theory",
            title: "Step 1: Immutable Bindings",
            conceptTitle: "Why Variables Don't Change by Default",
            conceptSummary: "When you write `x := 10`, L++ locks `x` to 10 so it cannot be accidentally changed.",
            explanationMarkdown: `In L++, variables declared with **\`:=\`** are **immutable** (cannot be reassigned).

\`\`\`
x := 10
# x = 20  <-- Error! x is immutable.
\`\`\`

This prevents accidental state bugs and race conditions in concurrent programs!`,
            codeExample: `def main() -> Void:
    x := 100
    print(x)`
          },
          {
            id: "step-1-3-2",
            type: "quiz",
            title: "Quick Check: Immutability",
            prompt: "By default, a variable created with `count := 5` is:",
            options: [
              { text: "Immutable (cannot be reassigned)", isCorrect: true },
              { text: "Mutable", isCorrect: false }
            ],
            explanation: "`:=` creates an immutable variable by default for software safety!"
          },
          {
            id: "step-1-3-3",
            type: "theory",
            title: "Step 2: Mutable Variables with `mut`",
            conceptTitle: "Allowing Reassignments",
            conceptSummary: "To make a variable changeable (like a score counter), add `mut` before its name.",
            explanationMarkdown: `When you explicitly want a variable to change value later, add **\`mut\`**:

\`\`\`
mut score := 10
score = score + 5  # Allowed! score is mutable.
\`\`\``,
            codeExample: `def main() -> Void:
    mut score := 10
    score = score + 50
    print(score)`
          },
          {
            id: "step-1-3-4",
            type: "code",
            title: "Practice: Create a Mutable Score",
            prompt: "Declare `mut score := 10`, add 5 to it, and print `score`.",
            initialCode: "def main() -> Void:\n    mut score := 10\n    score = score + 5\n    print(score)",
            solutionCode: "def main() -> Void:\n    mut score := 10\n    score = score + 5\n    print(score)",
            expectedOutput: "15",
            explanation: "`mut score := 10` allows `score = score + 5` to update the value to 15."
          }
        ]
      }
    ]
  },
  {
    id: "stage-2",
    title: "2. Safe Systems Memory & CPtr",
    level: "Advanced",
    icon: "🛡️",
    description: "Master CPtr fat pointers, bounds checking, and memory safety without unsafe code.",
    lessons: [
      {
        id: "lesson-2-1",
        title: "Safe C Memory Allocation (`CPtr`)",
        description: "Allocating checked C memory buffers with stdlib/c_memory.",
        xpReward: 45,
        steps: [
          {
            id: "step-2-1-1",
            type: "theory",
            title: "Step 1: C Pointers Without Crashes",
            conceptTitle: "What is `CPtr`?",
            conceptSummary: "L++ provides safe C pointer manipulation without `unsafe` blocks using `CPtr` fat pointers.",
            explanationMarkdown: `In standard C, pointers often cause segfaults and memory corruption. L++ solves this with **\`CPtr\`** fat pointers in \`stdlib/c_memory.lpp\`!

A **\`CPtr\`** tracks bounds and generation IDs so out-of-bounds accesses trigger clean L++ diagnostic panics instead of OS crashes!`,
            codeExample: `import c_memory

def main() -> Void:
    mem := c_memory_new(16)
    ptr := c_malloc(mem, 32)
    c_store_u32(ptr, 999)
    print(c_load_u32(ptr))
    c_free(ptr)
    c_memory_destroy(mem)`
          },
          {
            id: "step-2-1-2",
            type: "quiz",
            title: "Quick Check: CPtr Safety",
            prompt: "What happens if a CPtr attempts to read out of bounds in L++?",
            options: [
              { text: "It raises a safe diagnostic panic with provenance tracking", isCorrect: true },
              { text: "OS segfault crash", isCorrect: false }
            ],
            explanation: "CPtr tracks allocation bounds and raises safe catchable diagnostics!"
          },
          {
            id: "step-2-1-3",
            type: "code",
            title: "Practice: Allocate Safe C Memory",
            prompt: "Allocate 32 bytes with `c_malloc`, store 999 with `c_store_u32`, and print it with `c_load_u32`.",
            initialCode: "import c_memory\n\ndef main() -> Void:\n    mem := c_memory_new(16)\n    ptr := c_malloc(mem, 32)\n    c_store_u32(ptr, 999)\n    print(c_load_u32(ptr))\n    c_free(ptr)\n    c_memory_destroy(mem)",
            solutionCode: "import c_memory\n\ndef main() -> Void:\n    mem := c_memory_new(16)\n    ptr := c_malloc(mem, 32)\n    c_store_u32(ptr, 999)\n    print(c_load_u32(ptr))\n    c_free(ptr)\n    c_memory_destroy(mem)",
            expectedOutput: "999",
            explanation: "`c_malloc` creates a checked fat pointer `CPtr`."
          }
        ]
      }
    ]
  }
];
