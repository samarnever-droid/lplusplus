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
    title: "1. Rocket Launch Pad & Basics 🚀",
    level: "Beginner",
    icon: "🚀",
    description: "Build space rocket launchers and learn native L++ entry points and printing.",
    lessons: [
      {
        id: "lesson-1-1",
        title: "Rocket Countdown & `def main()`",
        description: "Launch your first L++ rocket into orbit!",
        xpReward: 25,
        steps: [
          {
            id: "step-1-1-1",
            type: "theory",
            title: "Step 1: The Rocket Ignition Point 🚀",
            conceptTitle: "Why Every L++ Rocket Needs `def main() -> Void:`",
            conceptSummary: "Imagine building a space rocket. The launch button is `def main() -> Void:`!",
            explanationMarkdown: `Welcome to L++ Academy! 🚀

L++ is an ultra-fast, native systems language that compiles directly into raw machine code (ELF/COFF executables).

Because your operating system needs to know **where the rocket launch button is**, every L++ program requires an entry point function:

\`\`\`
def main() -> Void:
    print_str("Ignition sequence start!")
\`\`\`

### Anatomy of Launch:
- **\`def main()\`**: The entry point function where execution starts.
- **\`-> Void\`**: Tells the compiler *"This function performs actions but returns no value back"*.
- **\`print_str("...")\`**: Hyper-fast native string emitter that writes text straight to the terminal!`,
            codeExample: `def main() -> Void:
    print_str("Rocket Launch 3.. 2.. 1.. FIRE! 🚀")`
          },
          {
            id: "step-1-1-2",
            type: "quiz",
            title: "Quick Check: Launch Rules 🛰️",
            prompt: "What happens if you try to press the launch button without `def main()` in L++?",
            options: [
              { text: "The L++ compiler raises error[E0002]: Expected 'def'", isCorrect: true },
              { text: "The rocket launches silently", isCorrect: false },
              { text: "It turns into JavaScript", isCorrect: false }
            ],
            explanation: "Spot on! L++ is a compiled native language — all executable code must be inside `def main() -> Void:`!"
          },
          {
            id: "step-1-1-3",
            type: "code",
            title: "Mission 1: Blast Off! 🛰️",
            prompt: "Inside `def main() -> Void:`, output 'Rocket Launch 3.. 2.. 1.. FIRE! 🚀' using `print_str`.",
            initialCode: "def main() -> Void:\n    # Launch your rocket below!\n    print_str(\"Rocket Launch 3.. 2.. 1.. FIRE! 🚀\")",
            solutionCode: "def main() -> Void:\n    print_str(\"Rocket Launch 3.. 2.. 1.. FIRE! 🚀\")",
            testCases: [
              { description: "Must contain 'def main() -> Void:'", expectedOutput: "Rocket Launch 3.. 2.. 1.. FIRE! 🚀" },
              { description: "Output must announce launch", expectedOutput: "Rocket Launch 3.. 2.. 1.. FIRE! 🚀" }
            ],
            hints: [
              "Start with `def main() -> Void:`.",
              "Indent 4 spaces and type `print_str(\"Rocket Launch 3.. 2.. 1.. FIRE! 🚀\")`."
            ],
            expectedOutput: "Rocket Launch 3.. 2.. 1.. FIRE! 🚀",
            explanation: "🎉 MISSION ACCOMPLISHED! Your rocket has safely entered orbit!"
          }
        ]
      },
      {
        id: "lesson-1-2",
        title: "RPG Hero Stats & Mutability 🗡️",
        description: "Build an RPG battle damage calculator with `:=` and `mut`.",
        xpReward: 30,
        steps: [
          {
            id: "step-1-2-1",
            type: "theory",
            title: "Step 1: The Immutable Shield 🛡️",
            conceptTitle: "Why Variables Are Locked by Default (`:=`)",
            conceptSummary: "In L++, variables declared with `:=` are locked like a bank vault so they can never be corrupted!",
            explanationMarkdown: `In RPG games, your hero's Base Max HP shouldn't accidentally change mid-fight!

In L++, when you write **\`base_hp := 100\`**, the value is **100% immutable**.

\`\`\`
base_hp := 100
# base_hp = 200  <-- ERROR! Shield activated: Immutable bindings cannot be reassigned!
\`\`\`

This guarantees 100% safety against accidental bugs!`,
            codeExample: `def main() -> Void:
    base_hp := 100
    print(base_hp)`
          },
          {
            id: "step-1-2-2",
            type: "theory",
            title: "Step 2: The Mutable Sword ⚔️",
            conceptTitle: "Unlocking State with `mut`",
            conceptSummary: "When your hero takes damage or gains EXP, use `mut` to allow variable updates!",
            explanationMarkdown: `When a variable *needs* to change (like Current Health or Gold), add the **\`mut\`** keyword!

\`\`\`
mut health := 100
health = health - 25  # Power Attack! Health drops to 75!
\`\`\`

Use **\`print(health)\`** to print numeric stats directly to the console!`,
            codeExample: `def main() -> Void:
    mut health := 100
    health = health - 25
    print(health)`
          },
          {
            id: "step-1-2-3",
            type: "code",
            title: "Mission 2: Dragon Boss Battle! 🐉",
            prompt: "Declare `mut hp := 100`, subtract 30 damage from `hp`, and print `hp`.",
            initialCode: "def main() -> Void:\n    mut hp := 100\n    hp = hp - 30\n    print(hp)",
            solutionCode: "def main() -> Void:\n    mut hp := 100\n    hp = hp - 30\n    print(hp)",
            testCases: [
              { description: "Hero HP must equal 70 after damage", expectedOutput: "70" }
            ],
            hints: [
              "Declare `mut hp := 100`.",
              "Subtract 30 with `hp = hp - 30`.",
              "Print using `print(hp)`."
            ],
            expectedOutput: "70",
            explanation: "⚔️ Critical Hit! Dragon dealt 30 damage, leaving Hero at 70 HP!"
          }
        ]
      }
    ]
  },
  {
    id: "block-2",
    title: "2. Cyberpunk Security Gates 🔐",
    level: "Intermediate",
    icon: "🔐",
    description: "Build secure door authentication systems with if/else conditionals & Stack structs.",
    lessons: [
      {
        id: "lesson-2-1",
        title: "Passcode Door Scanner",
        description: "Validate access passcodes with Pythonic if/else branching.",
        xpReward: 35,
        steps: [
          {
            id: "step-2-1-1",
            type: "theory",
            title: "Step 1: Cyberpunk Gate Logic ⚡",
            conceptTitle: "If, Elif, and Else Conditionals",
            conceptSummary: "Control security access gates with colons and indentation.",
            explanationMarkdown: `In Cyberpunk City, security gates scan employee passcodes:

\`\`\`
if passcode == 777:
    print_str("Access Granted! Welcome Officer.")
else:
    print_str("ACCESS DENIED! Intruder Alert!")
\`\`\``,
            codeExample: `def main() -> Void:
    passcode := 777
    if passcode == 777:
        print_str("Access Granted!")`
          },
          {
            id: "step-2-1-2",
            type: "code",
            title: "Mission 3: Hack the Security Gate! 🔓",
            prompt: "Check if `passcode == 777`. If true, print 'Access Granted!'.",
            initialCode: "def main() -> Void:\n    passcode := 777\n    if passcode == 777:\n        print_str(\"Access Granted!\")",
            solutionCode: "def main() -> Void:\n    passcode := 777\n    if passcode == 777:\n        print_str(\"Access Granted!\")",
            testCases: [
              { description: "Output must equal 'Access Granted!'", expectedOutput: "Access Granted!" }
            ],
            hints: [
              "Write `if passcode == 777:`.",
              "Print using `print_str(\"Access Granted!\")`."
            ],
            expectedOutput: "Access Granted!",
            explanation: "🔓 DOOR UNLOCKED! Welcome to the High-Tech Vault!"
          }
        ]
      }
    ]
  },
  {
    id: "block-3",
    title: "3. Safe Systems Memory & CPtr 🛡️",
    level: "Advanced",
    icon: "🛡️",
    description: "Master CPtr fat pointers, bounds checking, and memory sanitizers without unsafe code.",
    lessons: [
      {
        id: "lesson-3-1",
        title: "Safe C Memory Arena (`CPtr`)",
        description: "Allocate checked C memory buffers with stdlib/c_memory.",
        xpReward: 50,
        steps: [
          {
            id: "step-3-1-1",
            type: "theory",
            title: "Step 1: Bulletproof C Memory 🛡️",
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
            title: "Mission 4: Allocate Secure Vault Memory! 💎",
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
            explanation: "💎 VAULT SECURED! Checked fat pointer `CPtr` successfully verified!"
          }
        ]
      }
    ]
  }
];
