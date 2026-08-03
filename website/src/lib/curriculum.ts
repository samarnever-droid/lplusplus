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
    description: "Master real L++ entry point grammar, print_str output, and explicit types.",
    lessons: [
      {
        id: "lesson-1-1",
        title: "Hello World & `def main()` Entry Point",
        description: "Learn mandatory `def main() -> Void:` structure and `print_str` output.",
        xpReward: 20,
        steps: [
          {
            id: "step-1-1-1",
            type: "theory",
            title: "Step 1: Real L++ Program Structure",
            conceptTitle: "Why `def main() -> Void:` is Mandatory in L++",
            conceptSummary: "Unlike Python, L++ is a compiled native language! All executable code MUST be inside `def main() -> Void:`.",
            explanationMarkdown: `Welcome to real L++! Because L++ compiles directly into high-speed native machine code (ELF / COFF), the compiler requires an explicit entry point function.

### 1. Mandatory Entry Point: \`def main() -> Void:\`
Naked statements like \`print("hello")\` at the top level will trigger a compiler error:
\`\`\`
error[E0002]: Expected 'def', 'struct', 'enum', 'import'
\`\`\`
All code must be placed inside **\`def main() -> Void:\`**!

### 2. Output Strings with \`print_str\`
To print text strings, use **\`print_str("Hello World")\`**. For numbers, use **\`print(42)\`**.`,
            codeExample: `def main() -> Void:
    print_str("hello World")`
          },
          {
            id: "step-1-1-2",
            type: "quiz",
            title: "Quick Check: Compiler Rules",
            prompt: "What happens if you write a top-level statement like `print(\"hello\")` outside of `def main()` in L++?",
            options: [
              { text: "L++ compiler raises error[E0002]: Expected 'def'", isCorrect: true },
              { text: "It runs normally like Python", isCorrect: false },
              { text: "It creates a global variable", isCorrect: false }
            ],
            explanation: "L++ is a compiled native language! All executable statements must be inside `def main() -> Void:`."
          },
          {
            id: "step-1-1-3",
            type: "code",
            title: "Project Step: Write a Valid L++ Hello World",
            prompt: "Enclose `print_str(\"hello World\")` inside `def main() -> Void:`.",
            initialCode: "def main() -> Void:\n    # Write your print_str statement below\n    print_str(\"hello World\")",
            solutionCode: "def main() -> Void:\n    print_str(\"hello World\")",
            testCases: [
              { description: "Must contain 'def main() -> Void:'", expectedOutput: "hello World" },
              { description: "Output must equal 'hello World'", expectedOutput: "hello World" }
            ],
            hints: [
              "Start with `def main() -> Void:`.",
              "Indent 4 spaces and write `print_str(\"hello World\")`."
            ],
            expectedOutput: "hello World",
            explanation: "`def main() -> Void:` creates the entry point, and `print_str(\"hello World\")` outputs the string."
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
            type: "code",
            title: "Project Step: Temperature Converter",
            prompt: "Inside `def main() -> Void:`, declare `mut celsius := 20`, calculate Fahrenheit `celsius * 2 + 30`, and print the result.",
            initialCode: "def main() -> Void:\n    mut celsius := 20\n    fahrenheit := celsius * 2 + 30\n    print(fahrenheit)",
            solutionCode: "def main() -> Void:\n    mut celsius := 20\n    fahrenheit := celsius * 2 + 30\n    print(fahrenheit)",
            testCases: [
              { description: "Output must equal 70", expectedOutput: "70" }
            ],
            hints: [
              "Write code inside `def main() -> Void:`.",
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
  }
];
