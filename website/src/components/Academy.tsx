import { useEffect, useState, useRef, ReactNode, KeyboardEvent } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { Heart, Flame, Star, Trophy, CheckCircle, XCircle, Play, Sparkles, Lightbulb, Lock, Terminal, FileCode } from "lucide-react";
import { CURRICULUM, Stage, Lesson, LessonStep } from "../lib/curriculum";
import { getUserProgress, saveUserProgress, UserProgress } from "../lib/db";
import { evaluateLppCode } from "../lib/evaluator";

const LPP_KEYWORDS = [
  "def main() -> Void:",
  "print_str(\"\")",
  "print()",
  "mut ",
  "struct ",
  "c_memory",
  "CPtr",
  "c_malloc()",
  "c_free()",
  "if ",
  "else:",
  "while ",
  "Void",
  "Int",
  "Str",
  "Bool"
];

function renderFormattedMarkdown(text: string): ReactNode {
  const lines = text.split("\n");
  const elements: ReactNode[] = [];

  lines.forEach((line, idx) => {
    const trimmed = line.trim();
    if (!trimmed) return;

    if (trimmed.startsWith("### ")) {
      elements.push(
        <h3 key={idx} className="text-base font-bold font-mono text-acid mt-4 mb-2 flex items-center gap-2">
          <span className="h-2 w-2 rounded-full bg-acid" />
          {trimmed.replace("### ", "")}
        </h3>
      );
    } else if (trimmed.startsWith("- ")) {
      const content = parseInlineMarkdown(trimmed.replace("- ", ""));
      elements.push(
        <li key={idx} className="ml-5 list-disc text-white/80 text-xs md:text-sm font-sans leading-relaxed my-1">
          {content}
        </li>
      );
    } else {
      const content = parseInlineMarkdown(trimmed);
      elements.push(
        <p key={idx} className="text-white/80 text-xs md:text-sm font-sans leading-relaxed my-2">
          {content}
        </p>
      );
    }
  });

  return <div className="space-y-1">{elements}</div>;
}

function parseInlineMarkdown(text: string): ReactNode {
  const parts = text.split(/(\*\*.*?\*\*|`.*?`)/g);
  return parts.map((part, i) => {
    if (part.startsWith("**") && part.endsWith("**")) {
      return <strong key={i} className="text-white font-bold">{part.slice(2, -2)}</strong>;
    } else if (part.startsWith("`") && part.endsWith("`")) {
      return <code key={i} className="bg-black/60 border border-white/10 text-acid px-1.5 py-0.5 rounded font-mono text-xs">{part.slice(1, -1)}</code>;
    }
    return part;
  });
}

export default function Academy() {
  const [progress, setProgress] = useState<UserProgress | null>(null);
  const [activeStage, setActiveStage] = useState<Stage>(CURRICULUM[0]);
  const [activeLesson, setActiveLesson] = useState<Lesson | null>(null);
  const [currentStepIdx, setCurrentStepIdx] = useState(0);
  
  // Workspace states
  const [selectedOption, setSelectedOption] = useState<number | null>(null);
  const [userCode, setUserCode] = useState("");
  const [codeOutput, setCodeOutput] = useState("");
  const [showHint, setShowHint] = useState(false);
  const [feedback, setFeedback] = useState<{ isCorrect: boolean; message: string } | null>(null);
  const [isCompletedModal, setIsCompletedModal] = useState(false);

  // Auto-suggestion state
  const [suggestions, setSuggestions] = useState<string[]>([]);
  const [selectedSuggestionIdx, setSelectedSuggestionIdx] = useState(0);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    getUserProgress().then(setProgress);
  }, []);

  if (!progress) {
    return <div className="p-12 text-center text-white/50 font-mono">Loading L++ Academy...</div>;
  }

  const startLesson = (lesson: Lesson) => {
    setActiveLesson(lesson);
    setCurrentStepIdx(0);
    setFeedback(null);
    setSelectedOption(null);
    setShowHint(false);
    setSuggestions([]);
    const firstStep = lesson.steps[0];
    setUserCode(firstStep.initialCode || "");
    setCodeOutput("");
    setIsCompletedModal(false);
  };

  /**
   * Auto-bracket, Auto-indent, Tab & Auto-suggestion key handler
   */
  const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    const textarea = textareaRef.current;
    if (!textarea) return;

    const { selectionStart, selectionEnd, value } = textarea;

    // Handle suggestion selection via Tab or Enter
    if (suggestions.length > 0 && (e.key === "Tab" || e.key === "Enter")) {
      e.preventDefault();
      const selected = suggestions[selectedSuggestionIdx];
      
      // Get current word
      const beforeCursor = value.slice(0, selectionStart);
      const lastWordMatch = beforeCursor.match(/([a-zA-Z0-9_]+)$/);
      const lastWord = lastWordMatch ? lastWordMatch[1] : "";
      
      const newBefore = beforeCursor.slice(0, beforeCursor.length - lastWord.length) + selected;
      const newValue = newBefore + value.slice(selectionEnd);
      
      setUserCode(newValue);
      setSuggestions([]);
      
      setTimeout(() => {
        textarea.selectionStart = textarea.selectionEnd = newBefore.length;
      }, 0);
      return;
    }

    // Auto-indent on Enter if previous line ends with ':'
    if (e.key === "Enter") {
      const lineStart = value.lastIndexOf("\n", selectionStart - 1) + 1;
      const currentLine = value.slice(lineStart, selectionStart);
      
      if (currentLine.trim().endsWith(":")) {
        e.preventDefault();
        const indentMatch = currentLine.match(/^\s*/);
        const indent = (indentMatch ? indentMatch[0] : "") + "    ";
        const newValue = value.slice(0, selectionStart) + "\n" + indent + value.slice(selectionEnd);
        setUserCode(newValue);
        setTimeout(() => {
          textarea.selectionStart = textarea.selectionEnd = selectionStart + 1 + indent.length;
        }, 0);
        return;
      }
    }

    // Tab key inserts 4 spaces
    if (e.key === "Tab") {
      e.preventDefault();
      const newValue = value.slice(0, selectionStart) + "    " + value.slice(selectionEnd);
      setUserCode(newValue);
      setTimeout(() => {
        textarea.selectionStart = textarea.selectionEnd = selectionStart + 4;
      }, 0);
      return;
    }

    // Auto-closing brackets & quotes
    const pairs: Record<string, string> = {
      "(": ")",
      "\"": "\"",
      "'": "'",
      "[": "]",
      "{": "}"
    };

    if (pairs[e.key]) {
      e.preventDefault();
      const closing = pairs[e.key];
      const newValue = value.slice(0, selectionStart) + e.key + closing + value.slice(selectionEnd);
      setUserCode(newValue);
      setTimeout(() => {
        textarea.selectionStart = textarea.selectionEnd = selectionStart + 1;
      }, 0);
      return;
    }
  };

  /**
   * Triggers L++ IntelliSense suggestions as the user types
   */
  const handleCodeChange = (val: string) => {
    setUserCode(val);
    const textarea = textareaRef.current;
    if (!textarea) return;

    const selectionStart = textarea.selectionStart;
    const beforeCursor = val.slice(0, selectionStart);
    const lastWordMatch = beforeCursor.match(/([a-zA-Z0-9_]+)$/);

    if (lastWordMatch && lastWordMatch[1].length >= 2) {
      const prefix = lastWordMatch[1].toLowerCase();
      const matches = LPP_KEYWORDS.filter((k) => k.toLowerCase().includes(prefix));
      setSuggestions(matches);
      setSelectedSuggestionIdx(0);
    } else {
      setSuggestions([]);
    }
  };

  const handleQuizSubmit = (step: LessonStep) => {
    if (selectedOption === null) return;
    const option = step.options![selectedOption];
    if (option.isCorrect) {
      setFeedback({ isCorrect: true, message: step.explanation || "Correct!" });
    } else {
      setFeedback({ isCorrect: false, message: "Not quite right! " + (step.explanation || "") });
      if (progress.hearts > 1) {
        const updated = { ...progress, hearts: progress.hearts - 1 };
        setProgress(updated);
        saveUserProgress(updated);
      }
    }
  };

  const handleCodeRun = (step: LessonStep) => {
    const res = evaluateLppCode(userCode);

    if (res.exitCode === 0 && res.stdout) {
      setCodeOutput(res.stdout);

      if (step.expectedOutput) {
        const cleanActual = res.stdout.replace(/\s+/g, " ").trim();
        const cleanExpected = step.expectedOutput.replace(/\s+/g, " ").trim();

        if (cleanActual.includes(cleanExpected)) {
          setFeedback({ isCorrect: true, message: "🎉 All test assertions passed! " + (step.explanation || "") });
        } else {
          setFeedback({ isCorrect: false, message: `Expected output '${step.expectedOutput}', but got '${res.stdout}'.` });
        }
      } else {
        setFeedback({ isCorrect: true, message: "Code executed successfully!" });
      }
    } else {
      setCodeOutput(res.stderr || "Runtime error evaluating code.");
      setFeedback({ isCorrect: false, message: "Check your code syntax and try again!" });
    }
  };

  const nextStep = () => {
    if (!activeLesson) return;
    if (currentStepIdx + 1 < activeLesson.steps.length) {
      const nextIdx = currentStepIdx + 1;
      setCurrentStepIdx(nextIdx);
      const nextS = activeLesson.steps[nextIdx];
      setSelectedOption(null);
      setUserCode(nextS.initialCode || "");
      setCodeOutput("");
      setFeedback(null);
      setShowHint(false);
      setSuggestions([]);
    } else {
      const isNew = !progress.completedLessons.includes(activeLesson.id);
      const newXp = isNew ? progress.xp + activeLesson.xpReward : progress.xp;
      const newCompleted = isNew ? [...progress.completedLessons, activeLesson.id] : progress.completedLessons;
      
      const updated: UserProgress = {
        ...progress,
        xp: newXp,
        completedLessons: newCompleted,
        streak: isNew ? progress.streak + 1 : progress.streak
      };
      setProgress(updated);
      saveUserProgress(updated);
      setIsCompletedModal(true);
    }
  };

  const currentStep = activeLesson?.steps[currentStepIdx];

  return (
    <section id="academy" className="relative py-10 px-4 md:px-8 max-w-7xl mx-auto">
      {/* Duolingo Header Bar */}
      <div className="mb-8 rounded-2xl border border-white/10 bg-ink-soft/60 backdrop-blur-xl p-6 flex flex-wrap items-center justify-between gap-6">
        <div className="flex items-center gap-3">
          <span className="text-3xl">🎓</span>
          <div>
            <h2 className="text-2xl font-bold font-mono tracking-tight text-white">L++ Academy</h2>
            <p className="text-xs font-mono text-white/50 font-sans">freeCodeCamp & Duolingo Hybrid Systems Certification Path</p>
          </div>
        </div>

        {/* Stats */}
        <div className="flex items-center gap-5 font-mono text-sm">
          <div className="flex items-center gap-2 rounded-xl bg-red-500/10 border border-red-500/20 px-3.5 py-1.5 text-red-400">
            <Heart className="h-4 w-4 fill-current" />
            <span className="font-bold">{progress.hearts}/5</span>
          </div>
          <div className="flex items-center gap-2 rounded-xl bg-orange-500/10 border border-orange-500/20 px-3.5 py-1.5 text-orange-400">
            <Flame className="h-4 w-4 fill-current" />
            <span className="font-bold">{progress.streak} Day Streak</span>
          </div>
          <div className="flex items-center gap-2 rounded-xl bg-yellow-500/10 border border-yellow-500/20 px-3.5 py-1.5 text-yellow-400">
            <Star className="h-4 w-4 fill-current" />
            <span className="font-bold">{progress.xp} XP</span>
          </div>
          <div className="flex items-center gap-2 rounded-xl bg-acid/10 border border-acid/20 px-3.5 py-1.5 text-acid">
            <Trophy className="h-4 w-4" />
            <span className="font-bold">{progress.completedLessons.length} Certs</span>
          </div>
        </div>
      </div>

      {/* Stage Skill Tree */}
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-8">
        {/* Left Navigation Tree */}
        <div className="space-y-4">
          <h3 className="font-mono text-xs uppercase tracking-widest text-white/40 mb-2">Certification Blocks</h3>
          {CURRICULUM.map((stage) => {
            const isActive = activeStage.id === stage.id;
            return (
              <button
                key={stage.id}
                onClick={() => setActiveStage(stage)}
                className={`w-full text-left p-5 rounded-xl border transition-all flex items-start gap-4 ${
                  isActive
                    ? "border-acid bg-acid/10 text-white shadow-lg shadow-acid/5"
                    : "border-white/10 bg-ink-soft/30 hover:border-white/20 text-white/70"
                }`}
              >
                <span className="text-3xl">{stage.icon}</span>
                <div>
                  <span className="font-mono text-xs font-semibold px-2 py-0.5 rounded bg-white/10 text-white/80">
                    {stage.level}
                  </span>
                  <h4 className="font-mono font-bold text-base mt-1 text-white">{stage.title}</h4>
                  <p className="text-xs text-white/50 mt-1 line-clamp-2">{stage.description}</p>
                </div>
              </button>
            );
          })}
        </div>

        {/* Right Stage Lessons */}
        <div className="lg:col-span-2 rounded-2xl border border-white/10 bg-ink-soft/40 p-6 md:p-8">
          <div className="flex items-center justify-between mb-6 pb-4 border-b border-white/10">
            <div>
              <span className="font-mono text-xs text-acid uppercase tracking-widest">{activeStage.level} Block</span>
              <h3 className="text-xl font-bold font-mono text-white mt-0.5">{activeStage.title}</h3>
            </div>
            <span className="text-4xl">{activeStage.icon}</span>
          </div>

          {/* Lessons Grid */}
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
            {activeStage.lessons.map((lesson, idx) => {
              const isCompleted = progress.completedLessons.includes(lesson.id);
              const isUnlocked = idx === 0 || progress.completedLessons.includes(activeStage.lessons[idx - 1]?.id);

              return (
                <div
                  key={lesson.id}
                  className={`p-5 rounded-xl border flex flex-col justify-between transition-all ${
                    isCompleted
                      ? "border-emerald-500/30 bg-emerald-500/5"
                      : isUnlocked
                      ? "border-acid/30 bg-acid/5"
                      : "border-white/5 bg-white/[0.02] opacity-60"
                  }`}
                >
                  <div>
                    <div className="flex items-center justify-between mb-2">
                      <span className="font-mono text-xs text-white/40">Lesson {idx + 1}</span>
                      {isCompleted ? (
                        <CheckCircle className="h-5 w-5 text-emerald-400" />
                      ) : isUnlocked ? (
                        <Sparkles className="h-5 w-5 text-acid" />
                      ) : (
                        <Lock className="h-5 w-5 text-white/30" />
                      )}
                    </div>
                    <h4 className="font-mono font-bold text-white text-base">{lesson.title}</h4>
                    <p className="text-xs text-white/60 mt-1">{lesson.description}</p>
                  </div>

                  <div className="mt-4 pt-3 border-t border-white/10 flex items-center justify-between">
                    <span className="font-mono text-xs text-yellow-400">+{lesson.xpReward} XP</span>
                    <button
                      disabled={!isUnlocked}
                      onClick={() => startLesson(lesson)}
                      className={`px-4 py-1.5 rounded-lg font-mono text-xs font-bold transition-all ${
                        isUnlocked
                          ? "bg-acid text-ink hover:brightness-110 shadow-md shadow-acid/20"
                          : "bg-white/10 text-white/40 cursor-not-allowed"
                      }`}
                    >
                      {isCompleted ? "Review Lesson" : "Start Project"}
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      </div>

      {/* Split Workspace Modal with Auto-Suggestions & Auto-Brackets */}
      <AnimatePresence>
        {activeLesson && currentStep && !isCompletedModal && (
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="fixed inset-0 z-50 bg-ink/95 backdrop-blur-2xl p-4 md:p-6 flex flex-col justify-between"
          >
            {/* Header & Step Progress Bar */}
            <div className="mb-4">
              <div className="flex items-center justify-between pb-2">
                <div className="flex items-center gap-3">
                  <FileCode className="h-5 w-5 text-acid" />
                  <span className="font-mono text-sm font-bold text-white">
                    {activeLesson.title} — Step {currentStepIdx + 1} of {activeLesson.steps.length}
                  </span>
                </div>
                <button
                  onClick={() => setActiveLesson(null)}
                  className="text-white/40 hover:text-white font-mono text-xs px-3 py-1 rounded-lg border border-white/10"
                >
                  ✕ Exit Workspace
                </button>
              </div>
              <div className="h-1.5 w-full bg-white/10 rounded-full overflow-hidden">
                <div
                  className="h-full bg-acid transition-all duration-300"
                  style={{ width: `${((currentStepIdx + 1) / activeLesson.steps.length) * 100}%` }}
                />
              </div>
            </div>

            {/* Split Screen Container */}
            <div className="grid grid-cols-1 lg:grid-cols-2 gap-6 flex-1 overflow-hidden">
              {/* Left Panel: Instructions */}
              <div className="rounded-2xl border border-white/15 bg-ink-soft p-6 flex flex-col justify-between overflow-y-auto space-y-5">
                <div className="space-y-4">
                  <div className="flex items-center justify-between border-b border-white/10 pb-3">
                    <span className="font-mono text-xs text-acid font-bold uppercase tracking-wider">Instructions</span>
                    {currentStep.hints && (
                      <button
                        onClick={() => setShowHint(!showHint)}
                        className="flex items-center gap-1.5 text-yellow-400 hover:underline font-mono text-xs"
                      >
                        <Lightbulb className="h-4 w-4" /> {showHint ? "Hide Hint" : "Get Hint"}
                      </button>
                    )}
                  </div>

                  <h3 className="text-lg font-bold font-mono text-white">{currentStep.title}</h3>

                  {currentStep.type === "theory" && (
                    <div className="space-y-4">
                      <p className="text-white/90 text-sm font-sans leading-relaxed">{currentStep.conceptSummary}</p>
                      <div className="rounded-xl border border-white/10 bg-white/[0.02] p-4 text-xs">
                        {renderFormattedMarkdown(currentStep.explanationMarkdown || "")}
                      </div>
                      {currentStep.codeExample && (
                        <div className="rounded-xl border border-white/10 bg-black/80 p-4 font-mono text-xs text-acid">
                          <pre>{currentStep.codeExample}</pre>
                        </div>
                      )}
                    </div>
                  )}

                  {(currentStep.type === "quiz" || currentStep.type === "code") && (
                    <div className="space-y-4">
                      <p className="text-white/90 text-sm font-sans leading-relaxed">{currentStep.prompt}</p>

                      {currentStep.testCases && (
                        <div className="rounded-xl border border-white/10 bg-white/[0.02] p-4 space-y-2">
                          <span className="font-mono text-xs text-white/50 block mb-1">Test Assertions:</span>
                          {currentStep.testCases.map((tc, idx) => (
                            <div key={idx} className="flex items-center gap-2 font-mono text-xs text-white/80">
                              <CheckCircle className="h-4 w-4 text-emerald-400 shrink-0" />
                              <span>{tc.description}</span>
                            </div>
                          ))}
                        </div>
                      )}

                      {showHint && currentStep.hints && (
                        <div className="rounded-xl border border-yellow-500/30 bg-yellow-500/10 p-4 text-yellow-300 font-mono text-xs space-y-1">
                          <span className="font-bold block">💡 Hint:</span>
                          {currentStep.hints.map((h, hIdx) => (
                            <div key={hIdx}>• {h}</div>
                          ))}
                        </div>
                      )}

                      {currentStep.type === "quiz" && (
                        <div className="space-y-2">
                          {currentStep.options?.map((opt, oIdx) => (
                            <button
                              key={oIdx}
                              onClick={() => setSelectedOption(oIdx)}
                              className={`w-full text-left p-3.5 rounded-xl border font-mono text-xs transition-all ${
                                selectedOption === oIdx
                                  ? "border-acid bg-acid/20 text-white font-bold"
                                  : "border-white/10 bg-white/5 text-white/80 hover:bg-white/10"
                              }`}
                            >
                              {opt.text}
                            </button>
                          ))}
                        </div>
                      )}
                    </div>
                  )}

                  {feedback && (
                    <div
                      className={`p-4 rounded-xl border font-mono text-xs flex items-start gap-3 ${
                        feedback.isCorrect
                          ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-300"
                          : "border-red-500/40 bg-red-500/10 text-red-300"
                      }`}
                    >
                      {feedback.isCorrect ? <CheckCircle className="h-5 w-5 shrink-0" /> : <XCircle className="h-5 w-5 shrink-0" />}
                      <div>{feedback.message}</div>
                    </div>
                  )}
                </div>

                <div className="pt-4 border-t border-white/10 flex justify-between items-center">
                  {currentStep.type === "theory" && (
                    <button
                      onClick={nextStep}
                      className="w-full py-3 rounded-xl bg-acid text-ink font-mono text-sm font-bold shadow-lg shadow-acid/20 hover:brightness-110 transition-all"
                    >
                      Continue to Exercise →
                    </button>
                  )}

                  {currentStep.type === "quiz" && (
                    <div className="flex items-center justify-between w-full">
                      <button
                        disabled={selectedOption === null}
                        onClick={() => handleQuizSubmit(currentStep)}
                        className="px-6 py-2.5 rounded-xl bg-white/10 hover:bg-white/20 text-white font-mono text-xs font-bold transition-all disabled:opacity-40"
                      >
                        Check Answer
                      </button>
                      {feedback?.isCorrect && (
                        <button
                          onClick={nextStep}
                          className="px-6 py-2.5 rounded-xl bg-acid text-ink font-mono text-xs font-bold shadow-lg shadow-acid/20 hover:brightness-110 transition-all"
                        >
                          Next Challenge →
                        </button>
                      )}
                    </div>
                  )}
                </div>
              </div>

              {/* Right Panel: Interactive Code Editor with IntelliSense & Auto-Brackets */}
              <div className="rounded-2xl border border-white/15 bg-black/80 flex flex-col justify-between overflow-hidden relative">
                {/* Top Bar */}
                <div className="flex items-center justify-between bg-white/5 px-4 py-2.5 border-b border-white/10 font-mono text-xs text-white/60">
                  <div className="flex items-center gap-2">
                    <span className="h-3 w-3 rounded-full bg-red-500/80" />
                    <span className="h-3 w-3 rounded-full bg-yellow-500/80" />
                    <span className="h-3 w-3 rounded-full bg-green-500/80" />
                    <span className="ml-2 text-white/80 font-bold">main.lpp</span>
                  </div>

                  <button
                    onClick={() => handleCodeRun(currentStep)}
                    className="flex items-center gap-2 px-4 py-1.5 rounded-lg bg-acid text-ink font-mono text-xs font-bold shadow-lg shadow-acid/30 hover:brightness-110 transition-all animate-pulse"
                  >
                    <Play className="h-3.5 w-3.5 fill-current" /> ▶ Run Code
                  </button>
                </div>

                {/* Editor Textarea */}
                <div className="flex-1 p-4 font-mono text-xs relative">
                  <textarea
                    ref={textareaRef}
                    value={userCode}
                    onKeyDown={handleKeyDown}
                    onChange={(e) => handleCodeChange(e.target.value)}
                    className="w-full h-full bg-transparent text-acid focus:outline-none resize-none font-mono leading-relaxed"
                    placeholder="// Write your L++ code here..."
                  />

                  {/* Auto-Suggestion IntelliSense Popup Menu */}
                  {suggestions.length > 0 && (
                    <div className="absolute bottom-6 left-6 z-50 rounded-xl border border-acid/40 bg-ink-soft/95 backdrop-blur-xl p-2 shadow-2xl space-y-1 font-mono text-xs">
                      <span className="text-[10px] text-white/40 uppercase tracking-widest block px-2 pb-1 border-b border-white/10">
                        L++ IntelliSense (Press Tab ↹)
                      </span>
                      {suggestions.slice(0, 5).map((sug, sIdx) => (
                        <div
                          key={sIdx}
                          className={`px-3 py-1.5 rounded-lg cursor-pointer transition-all flex items-center justify-between gap-4 ${
                            sIdx === selectedSuggestionIdx ? "bg-acid text-ink font-bold" : "text-white/80 hover:bg-white/10"
                          }`}
                        >
                          <span>{sug}</span>
                          <span className="text-[10px] opacity-60">Keyword</span>
                        </div>
                      ))}
                    </div>
                  )}
                </div>

                {/* Live Console Output Bar */}
                <div className="border-t border-white/10 bg-black/90 p-4 space-y-3">
                  <div className="flex items-center justify-between font-mono text-xs text-white/40">
                    <span className="flex items-center gap-1.5">
                      <Terminal className="h-3.5 w-3.5 text-acid" /> Real Terminal Output
                    </span>
                    <button
                      onClick={() => handleCodeRun(currentStep)}
                      className="flex items-center gap-1.5 px-3 py-1 rounded bg-white/10 text-white/80 hover:bg-white/20 transition-all"
                    >
                      <Play className="h-3 w-3" /> Execute Output
                    </button>
                  </div>

                  <div className="min-h-[70px] rounded-xl bg-white/[0.03] p-3 font-mono text-xs text-emerald-400 whitespace-pre-wrap">
                    {codeOutput ? codeOutput : <span className="text-white/30 italic">Click '▶ Run Code' above to execute your code...</span>}
                  </div>

                  {feedback?.isCorrect && currentStep.type === "code" && (
                    <button
                      onClick={nextStep}
                      className="w-full py-2.5 rounded-xl bg-acid text-ink font-mono text-xs font-bold shadow-lg shadow-acid/20 hover:brightness-110 transition-all"
                    >
                      All Tests Passed! Proceed →
                    </button>
                  )}
                </div>
              </div>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Completion Celebration Modal */}
      <AnimatePresence>
        {isCompletedModal && activeLesson && (
          <motion.div
            initial={{ scale: 0.9, opacity: 0 }}
            animate={{ scale: 1, opacity: 1 }}
            className="fixed inset-0 z-50 bg-ink/90 backdrop-blur-2xl p-4 flex items-center justify-center"
          >
            <div className="w-full max-w-md rounded-2xl border border-acid/40 bg-ink-soft p-8 text-center space-y-6 shadow-2xl">
              <span className="text-6xl animate-bounce inline-block">🎉</span>
              <h3 className="text-2xl font-bold font-mono text-white">Lesson Completed!</h3>
              <p className="text-sm font-mono text-white/70">
                You earned <span className="text-yellow-400 font-bold">+{activeLesson.xpReward} XP</span> and completed your L++ project step!
              </p>
              <button
                onClick={() => {
                  setIsCompletedModal(false);
                  setActiveLesson(null);
                }}
                className="w-full py-3 rounded-xl bg-acid text-ink font-mono text-sm font-bold shadow-lg shadow-acid/20 hover:brightness-110 transition-all"
              >
                Back to Academy
              </button>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </section>
  );
}
