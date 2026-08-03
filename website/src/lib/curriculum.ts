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

export interface Lesson {
  id: string;
  title: string;
  description: string;
  xpReward: number;
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
        title: "Hello L++",
        description: "Write your first L++ program with print_str.",
        xpReward: 20,
        challenges: [
          {
            id: "c-1-1-1",
            title: "First Steps",
            type: "quiz",
            prompt: "Which keyword defines a top-level function in L++?",
            options: [
              { text: "def", isCorrect: true },
              { text: "func", isCorrect: false },
              { text: "function", isCorrect: false },
              { text: "fn", isCorrect: false }
            ],
            explanation: "In L++, functions are defined using the explicit `def` keyword!"
          },
          {
            id: "c-1-1-2",
            title: "Print Hello",
            type: "code",
            prompt: "Complete the function to output 'Hello L++!' using print_str.",
            initialCode: "def main() -> Void:\n    # Write your code below\n    print_str(\"...\")",
            solutionCode: "def main() -> Void:\n    print_str(\"Hello L++!\")",
            expectedOutput: "Hello L++!",
            explanation: "`print_str(\"Hello L++!\")` outputs raw string literals to the console."
          }
        ]
      },
      {
        id: "lesson-1-2",
        title: "Variables & Mutability",
        description: "Master immutable bindings and `mut` reassignments.",
        xpReward: 25,
        challenges: [
          {
            id: "c-1-2-1",
            title: "Mutability Rule",
            type: "quiz",
            prompt: "By default, variables declared with `x := 10` are:",
            options: [
              { text: "Immutable (cannot be reassigned)", isCorrect: true },
              { text: "Mutable", isCorrect: false },
              { text: "Global", isCorrect: false }
            ],
            explanation: "L++ enforces memory safety by making bindings immutable by default unless declared with `mut x := 10`!"
          },
          {
            id: "c-1-2-2",
            title: "Mutable Counter",
            type: "code",
            prompt: "Declare a mutable integer `count`, increment it by 5, and print it.",
            initialCode: "def main() -> Void:\n    mut count := 10\n    count = count + 5\n    print(count)",
            solutionCode: "def main() -> Void:\n    mut count := 10\n    count = count + 5\n    print(count)",
            expectedOutput: "15",
            explanation: "`mut` allows reassigning bindings via `=`. `print(count)` outputs `15`."
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
        title: "If & Else Conditions",
        description: "Master Python-style indent conditionals with explicit types.",
        xpReward: 30,
        challenges: [
          {
            id: "c-2-1-1",
            title: "Branching Output",
            type: "code",
            prompt: "Write an if-statement checking if `age >= 18` and print 'Adult'.",
            initialCode: "def main() -> Void:\n    age := 20\n    if age >= 18:\n        print_str(\"Adult\")",
            solutionCode: "def main() -> Void:\n    age := 20\n    if age >= 18:\n        print_str(\"Adult\")",
            expectedOutput: "Adult",
            explanation: "L++ uses pythonic colon `:` syntax for clean block structuring."
          }
        ]
      },
      {
        id: "lesson-2-2",
        title: "Struct Value Types",
        description: "Zero-overhead stack-allocated data structures.",
        xpReward: 35,
        challenges: [
          {
            id: "c-2-2-1",
            title: "Struct Declaration",
            type: "code",
            prompt: "Define a Point struct with x: Int and y: Int.",
            initialCode: "struct Point:\n    x: Int\n    y: Int\n\ndef main() -> Void:\n    p := Point(10, 20)\n    print(p.x + p.y)",
            solutionCode: "struct Point:\n    x: Int\n    y: Int\n\ndef main() -> Void:\n    p := Point(10, 20)\n    print(p.x + p.y)",
            expectedOutput: "30",
            explanation: "Structs in L++ are allocated directly on the Stack by default with zero GC/ARC overhead!"
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
        title: "CPtr Memory Allocation",
        description: "Safe checked C pointer allocations via stdlib/c_memory.",
        xpReward: 45,
        challenges: [
          {
            id: "c-3-1-1",
            title: "Safe C Pointer",
            type: "quiz",
            prompt: "What happens if a CPtr accesses memory out of bounds in L++?",
            options: [
              { text: "L++ raises a safe panic diagnostic with provenance tracking", isCorrect: true },
              { text: "OS segfault crash", isCorrect: false },
              { text: "Silent memory corruption", isCorrect: false }
            ],
            explanation: "CPtr tracks allocation bounds and subobject offset bounds, turning raw C pointer bugs into 100% safe catchable diagnostics!"
          },
          {
            id: "c-3-1-2",
            title: "Alloc & Free",
            type: "code",
            prompt: "Allocate 32 bytes using c_memory and c_malloc.",
            initialCode: "import c_memory\n\ndef main() -> Void:\n    mem := c_memory_new(16)\n    ptr := c_malloc(mem, 32)\n    c_store_u32(ptr, 999)\n    print(c_load_u32(ptr))\n    c_free(ptr)\n    c_memory_destroy(mem)",
            solutionCode: "import c_memory\n\ndef main() -> Void:\n    mem := c_memory_new(16)\n    ptr := c_malloc(mem, 32)\n    c_store_u32(ptr, 999)\n    print(c_load_u32(ptr))\n    c_free(ptr)\n    c_memory_destroy(mem)",
            expectedOutput: "999",
            explanation: "c_malloc creates a checked fat pointer CPtr with bounds tracking."
          }
        ]
      }
    ]
  },
  {
    id: "stage-4",
    title: "4. Master Architect",
    level: "Master",
    icon: "👑",
    description: "Build high-performance concurrent, FFI, and self-hosted systems.",
    lessons: [
      {
        id: "lesson-4-1",
        title: "Concurrent Task Spawning",
        description: "Spawn background threads with automatic lockless ARC promotion.",
        xpReward: 50,
        challenges: [
          {
            id: "c-4-1-1",
            title: "Task Spawning",
            type: "code",
            prompt: "Launch a concurrent background task with spawn.",
            initialCode: "def main() -> Void:\n    spawn fn():\n        print_str(\"Background Worker\")\n    print_str(\"Main Thread\")",
            solutionCode: "def main() -> Void:\n    spawn fn():\n        print_str(\"Background Worker\")\n    print_str(\"Main Thread\")",
            expectedOutput: "Main Thread",
            explanation: "L++ compiles closures and automatically demotes single-threaded variables to plain stack values!"
          }
        ]
      }
    ]
  }
];
