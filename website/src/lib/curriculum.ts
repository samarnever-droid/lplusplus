export interface QuizOption {
  text: string;
  isCorrect: boolean;
}

export type StepType = "theory" | "quiz" | "code";

export interface LessonStep {
  id: string;
  type: StepType;
  title: string;
  conceptTitle?: string;
  conceptSummary?: string;
  explanationMarkdown?: string;
  codeExample?: string;
  prompt?: string;
  options?: QuizOption[];
  initialCode?: string;
  solutionCode?: string;
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
            type: "theory",
            title: "Step 2: Understanding `def main() -> Void:`",
            conceptTitle: "Deconstructing `def main() -> Void:`",
            conceptSummary: "Why do we write `def main() -> Void:`? Let's break down every part!",
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
            title: "Practice: Print Your First Message",
            prompt: "Complete the code to print 'Hello World' using `print`.",
            initialCode: "def main() -> Void:\n    # Write your print statement below\n    print(\"Hello World\")",
            solutionCode: "def main() -> Void:\n    print(\"Hello World\")",
            expectedOutput: "Hello World",
            explanation: "`print(\"Hello World\")` outputs raw string text to the screen."
          }
        ]
      },
      {
        id: "lesson-1-2",
        title: "Variables (`:=` vs `mut`)",
        description: "Understand immutability and state changes.",
        xpReward: 30,
        steps: [
          {
            id: "step-1-2-1",
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
            id: "step-1-2-2",
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
            id: "step-1-2-3",
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
            id: "step-1-2-4",
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
  }
];
