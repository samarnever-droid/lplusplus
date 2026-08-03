export interface QuizOption {
  text: string;
  isCorrect: boolean;
}

export type StepType = "theory" | "quiz" | "code";

export interface TestCase {
  description: string;
  expectedOutput: string;
}

export interface LessonStep {
  id: string;
  type: StepType;
  title: string;
  conceptTitle?: string;
  conceptSummary?: string;
  explanationMarkdown?: string;
  codeExample?: string;
  hints?: string[];
  prompt?: string;
  options?: QuizOption[];
  initialCode?: string;
  solutionCode?: string;
  testCases?: TestCase[];
  expectedOutput?: string;
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
    id: "block-1",
    title: "1. Scientific Computing & Basics",
    level: "Beginner",
    icon: "🐍",
    description: "Master Pythonic printing, functions, variables, and build your first calculator.",
    lessons: [
      {
        id: "lesson-1-1",
        title: "Hello World & Printing",
        description: "Start printing with Pythonic simplicity.",
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
            type: "theory",
            title: "Step 2: Deconstructing `def main() -> Void:`",
            conceptTitle: "Why `def main() -> Void:` is used",
            conceptSummary: "Let's break down every part of the entry point function!",
            explanationMarkdown: `When building native compiled programs, the computer needs to know where execution begins.

Here is what every part of **\`def main() -> Void:\`** means:

### 1. Why \`def main()\` is required?
The operating system needs an **entry point** function. \`def main()\` tells the CPU *"Start running my program here!"*

### 2. What does \`->\` mean?
The **\`->\`** arrow is the **Return Type Indicator**. It tells the compiler what type of result the function will give back (e.g. \`-> Int\` for numbers, \`-> Str\` for text).

### 3. What does \`Void\` mean?
**\`Void\`** means **"Nothing" / "No return value"**. Since \`main()\` just prints text and doesn't calculate a return number, its return type is \`Void\`!`,
            codeExample: `# Function returns an Int
def add(a: Int, b: Int) -> Int:
    return a + b

# Function returns Void (nothing)
def main() -> Void:
    print("Hello World")`
          },
          {
            id: "step-1-1-4",
            type: "quiz",
            title: "Quick Check: `-> Void`",
            prompt: "In `def main() -> Void:`, what does `Void` stand for?",
            options: [
              { text: "The function returns nothing (no return value)", isCorrect: true },
              { text: "The function returns an integer", isCorrect: false },
              { text: "It means the function is empty", isCorrect: false }
            ],
            explanation: "`Void` means the function performs actions but produces no return value."
          },
          {
            id: "step-1-1-5",
            type: "code",
            title: "Project Step: Print Your First Message",
            prompt: "Complete the code to print 'Hello World' using `print`.",
            initialCode: "def main() -> Void:\n    # Write your print statement below\n    print(\"Hello World\")",
            solutionCode: "def main() -> Void:\n    print(\"Hello World\")",
            testCases: [
              { description: "Output must equal 'Hello World'", expectedOutput: "Hello World" }
            ],
            hints: [
              "Use `print(\"Hello World\")` inside `main()`.",
              "Make sure to capitalize 'Hello World' correctly."
            ],
            expectedOutput: "Hello World",
            explanation: "`print(\"Hello World\")` outputs raw string text to the screen."
          }
        ]
      },
      {
        id: "lesson-1-2",
        title: "Variables & Calculations",
        description: "Master immutable bindings and `mut` reassignments.",
        xpReward: 25,
        steps: [
          {
            id: "step-1-2-1",
            type: "theory",
            title: "Step 1: Immutable Bindings (`:=`)",
            conceptTitle: "Why Variables Don't Change by Default",
            conceptSummary: "When you write `x := 10`, L++ locks `x` to 10 so it cannot be accidentally changed.",
            explanationMarkdown: `In L++, variables declared with **\`:=\`** are **immutable** (cannot be reassigned).

\`\`\`
x := 10
# x = 20  <-- Error! x is immutable.
\`\`\`

This prevents accidental state bugs and race conditions!`,
            codeExample: `def main() -> Void:
    x := 100
    print(x)`
          },
          {
            id: "step-1-2-2",
            type: "theory",
            title: "Step 2: Mutable Variables (`mut`)",
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
            id: "step-1-2-3",
            type: "code",
            title: "Project Step: Temperature Converter",
            prompt: "Declare `mut celsius := 20`, calculate Fahrenheit `celsius * 2 + 30`, and print the result.",
            initialCode: "def main() -> Void:\n    mut celsius := 20\n    fahrenheit := celsius * 2 + 30\n    print(fahrenheit)",
            solutionCode: "def main() -> Void:\n    mut celsius := 20\n    fahrenheit := celsius * 2 + 30\n    print(fahrenheit)",
            testCases: [
              { description: "Output must equal 70", expectedOutput: "70" }
            ],
            hints: [
              "Declare `mut celsius := 20`.",
              "Calculate `fahrenheit := celsius * 2 + 30`.",
              "Print `fahrenheit` with `print(fahrenheit)`."
            ],
            expectedOutput: "70",
            explanation: "`celsius * 2 + 30` evaluates to `70`."
          }
        ]
      }
    ]
  },
  {
    id: "block-2",
    title: "2. Control Flow & Structs",
    level: "Intermediate",
    icon: "⚡",
    description: "Branching with if/else, while loops, and zero-overhead Stack structs.",
    lessons: [
      {
        id: "lesson-2-1",
        title: "If / Else Conditionals",
        description: "Branching execution with boolean logic.",
        xpReward: 30,
        steps: [
          {
            id: "step-2-1-1",
            type: "theory",
            title: "Step 1: Pythonic Indented Branching",
            conceptTitle: "If, Elif, and Else",
            conceptSummary: "Control program flow with colons and indentation.",
            explanationMarkdown: `Use **\`if\`**, **\`elif\`**, and **\`else\`** to run code conditionally:

\`\`\`
if age >= 18:
    print("Adult")
else:
    print("Minor")
\`\`\``,
            codeExample: `def main() -> Void:
    age := 20
    if age >= 18:
        print("Adult")`
          },
          {
            id: "step-2-1-2",
            type: "code",
            title: "Project Step: Eligibility Checker",
            prompt: "Check if `score >= 50`. If so, print 'Pass'.",
            initialCode: "def main() -> Void:\n    score := 75\n    if score >= 50:\n        print(\"Pass\")",
            solutionCode: "def main() -> Void:\n    score := 75\n    if score >= 50:\n        print(\"Pass\")",
            testCases: [
              { description: "Output must equal 'Pass'", expectedOutput: "Pass" }
            ],
            hints: [
              "Write `if score >= 50:`.",
              "Indent `print(\"Pass\")` under the if block."
            ],
            expectedOutput: "Pass",
            explanation: "`score >= 50` evaluates true and prints 'Pass'."
          }
        ]
      }
    ]
  },
  {
    id: "block-3",
    title: "3. Safe Systems Memory & CPtr",
    level: "Advanced",
    icon: "🛡️",
    description: "Master CPtr fat pointers, bounds checking, and memory safety without unsafe code.",
    lessons: [
      {
        id: "lesson-3-1",
        title: "Safe C Memory Allocation (`CPtr`)",
        description: "Allocating checked C memory buffers with stdlib/c_memory.",
        xpReward: 45,
        steps: [
          {
            id: "step-3-1-1",
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
            id: "step-3-1-2",
            type: "code",
            title: "Project Step: Allocate Safe C Memory",
            prompt: "Allocate 32 bytes with `c_malloc`, store 999 with `c_store_u32`, and print it with `c_load_u32`.",
            initialCode: "import c_memory\n\ndef main() -> Void:\n    mem := c_memory_new(16)\n    ptr := c_malloc(mem, 32)\n    c_store_u32(ptr, 999)\n    print(c_load_u32(ptr))\n    c_free(ptr)\n    c_memory_destroy(mem)",
            solutionCode: "import c_memory\n\ndef main() -> Void:\n    mem := c_memory_new(16)\n    ptr := c_malloc(mem, 32)\n    c_store_u32(ptr, 999)\n    print(c_load_u32(ptr))\n    c_free(ptr)\n    c_memory_destroy(mem)",
            testCases: [
              { description: "Output must equal 999", expectedOutput: "999" }
            ],
            hints: [
              "Use `c_malloc(mem, 32)`.",
              "Use `c_store_u32(ptr, 999)`.",
              "Print using `print(c_load_u32(ptr))`."
            ],
            expectedOutput: "999",
            explanation: "`c_malloc` creates a checked fat pointer `CPtr`."
          }
        ]
      }
    ]
  }
];
