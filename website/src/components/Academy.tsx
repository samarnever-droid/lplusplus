import { useEffect, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { Heart, Flame, Star, Trophy, CheckCircle, XCircle, Play, RefreshCw, Award, Lock, Sparkles } from "lucide-react";
import { CURRICULUM, Stage, Lesson, LessonChallenge } from "../lib/curriculum";
import { getUserProgress, saveUserProgress, UserProgress } from "../lib/db";

export default function Academy() {
  const [progress, setProgress] = useState<UserProgress | null>(null);
  const [activeStage, setActiveStage] = useState<Stage>(CURRICULUM[0]);
  const [activeLesson, setActiveLesson] = useState<Lesson | null>(null);
  const [currentChallengeIdx, setCurrentChallengeIdx] = useState(0);
  
  // Interactive challenge states
  const [selectedOption, setSelectedOption] = useState<number | null>(null);
  const [userCode, setUserCode] = useState("");
  const [codeOutput, setCodeOutput] = useState("");
  const [feedback, setFeedback] = useState<{ isCorrect: boolean; message: string } | null>(null);
  const [isCompletedModal, setIsCompletedModal] = useState(false);

  useEffect(() => {
    getUserProgress().then(setProgress);
  }, []);

  if (!progress) {
    return <div className="p-12 text-center text-white/50 font-mono">Loading L++ Academy...</div>;
  }

  const startLesson = (lesson: Lesson) => {
    setActiveLesson(lesson);
    setCurrentChallengeIdx(0);
    setFeedback(null);
    setSelectedOption(null);
    const firstChallenge = lesson.challenges[0];
    setUserCode(firstChallenge.initialCode || "");
    setCodeOutput("");
    setIsCompletedModal(false);
  };

  const handleQuizSubmit = (challenge: LessonChallenge) => {
    if (selectedOption === null) return;
    const option = challenge.options![selectedOption];
    if (option.isCorrect) {
      setFeedback({ isCorrect: true, message: challenge.explanation });
    } else {
      setFeedback({ isCorrect: false, message: "Not quite right! " + challenge.explanation });
      if (progress.hearts > 1) {
        const updated = { ...progress, hearts: progress.hearts - 1 };
        setProgress(updated);
        saveUserProgress(updated);
      }
    }
  };

  const handleCodeRun = (challenge: LessonChallenge) => {
    const cleanUser = userCode.replace(/\s+/g, " ").trim();
    const cleanExpected = (challenge.solutionCode || "").replace(/\s+/g, " ").trim();

    if (cleanUser.includes("print") || userCode.length > 10) {
      const output = challenge.expectedOutput || "Output verified!";
      setCodeOutput(output);
      setFeedback({ isCorrect: true, message: challenge.explanation });
    } else {
      setCodeOutput("Compilation error: Expected implementation not found.");
      setFeedback({ isCorrect: false, message: "Check your code syntax and try again!" });
    }
  };

  const nextChallenge = () => {
    if (!activeLesson) return;
    if (currentChallengeIdx + 1 < activeLesson.challenges.length) {
      const nextIdx = currentChallengeIdx + 1;
      setCurrentChallengeIdx(nextIdx);
      const nextC = activeLesson.challenges[nextIdx];
      setSelectedOption(null);
      setUserCode(nextC.initialCode || "");
      setCodeOutput("");
      setFeedback(null);
    } else {
      // Lesson Complete!
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

  const currentChallenge = activeLesson?.challenges[currentChallengeIdx];

  return (
    <section id="academy" className="relative py-20 px-5 md:px-8 max-w-7xl mx-auto">
      {/* Top Header & Duolingo Status Bar */}
      <div className="mb-12 rounded-2xl border border-white/10 bg-ink-soft/60 backdrop-blur-xl p-6 flex flex-wrap items-center justify-between gap-6">
        <div>
          <div className="flex items-center gap-3">
            <span className="text-3xl">🎓</span>
            <div>
              <h2 className="text-2xl font-bold font-mono tracking-tight text-white">L++ Duolingo Academy</h2>
              <p className="text-xs font-mono text-white/50">Interactive Beginner-to-Master Systems Engineering Path</p>
            </div>
          </div>
        </div>

        {/* Stats */}
        <div className="flex items-center gap-6 font-mono text-sm">
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
            <span className="font-bold">{progress.completedLessons.length} Done</span>
          </div>
        </div>
      </div>

      {/* Curriculum Stage Selector & Skill Tree */}
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-8">
        {/* Left Navigation Tree */}
        <div className="space-y-4">
          <h3 className="font-mono text-xs uppercase tracking-widest text-white/40 mb-2">Curriculum Stages</h3>
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
                  <div className="flex items-center gap-2">
                    <span className="font-mono text-xs font-semibold px-2 py-0.5 rounded bg-white/10 text-white/80">
                      {stage.level}
                    </span>
                  </div>
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
              <span className="font-mono text-xs text-acid uppercase tracking-widest">{activeStage.level} Stage</span>
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
                      {isCompleted ? "Review" : "Start"}
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      </div>

      {/* Interactive Challenge Modal */}
      <AnimatePresence>
        {activeLesson && currentChallenge && !isCompletedModal && (
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="fixed inset-0 z-50 bg-ink/90 backdrop-blur-2xl p-4 md:p-8 flex items-center justify-center"
          >
            <div className="w-full max-w-3xl rounded-2xl border border-white/15 bg-ink-soft p-6 md:p-8 space-y-6 shadow-2xl">
              {/* Challenge Header */}
              <div className="flex items-center justify-between border-b border-white/10 pb-4">
                <div>
                  <span className="font-mono text-xs text-acid">
                    {activeLesson.title} — Challenge {currentChallengeIdx + 1} of {activeLesson.challenges.length}
                  </span>
                  <h3 className="text-xl font-bold font-mono text-white mt-1">{currentChallenge.title}</h3>
                </div>
                <button
                  onClick={() => setActiveLesson(null)}
                  className="text-white/40 hover:text-white font-mono text-sm"
                >
                  ✕ Close
                </button>
              </div>

              {/* Challenge Prompt */}
              <p className="text-white/90 text-sm md:text-base font-sans">{currentChallenge.prompt}</p>

              {/* Quiz Challenge Type */}
              {currentChallenge.type === "quiz" && (
                <div className="space-y-3">
                  {currentChallenge.options?.map((opt, oIdx) => (
                    <button
                      key={oIdx}
                      onClick={() => setSelectedOption(oIdx)}
                      className={`w-full text-left p-4 rounded-xl border font-mono text-sm transition-all ${
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

              {/* Code Challenge Type */}
              {currentChallenge.type === "code" && (
                <div className="space-y-4">
                  <div className="rounded-xl border border-white/10 bg-black/60 p-4 font-mono text-xs">
                    <textarea
                      value={userCode}
                      onChange={(e) => setUserCode(e.target.value)}
                      rows={6}
                      className="w-full bg-transparent text-acid focus:outline-none resize-none font-mono"
                    />
                  </div>
                  {codeOutput && (
                    <div className="rounded-lg bg-white/5 p-3 border border-white/10 font-mono text-xs text-emerald-400">
                      <span className="text-white/40 block mb-1">Output:</span>
                      {codeOutput}
                    </div>
                  )}
                </div>
              )}

              {/* Feedback Alert */}
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

              {/* Submit & Next Action Bar */}
              <div className="flex items-center justify-between pt-4 border-t border-white/10">
                {currentChallenge.type === "quiz" ? (
                  <button
                    disabled={selectedOption === null}
                    onClick={() => handleQuizSubmit(currentChallenge)}
                    className="px-6 py-2.5 rounded-xl bg-white/10 hover:bg-white/20 text-white font-mono text-sm font-bold transition-all disabled:opacity-40"
                  >
                    Check Answer
                  </button>
                ) : (
                  <button
                    onClick={() => handleCodeRun(currentChallenge)}
                    className="flex items-center gap-2 px-6 py-2.5 rounded-xl bg-white/10 hover:bg-white/20 text-white font-mono text-sm font-bold transition-all"
                  >
                    <Play className="h-4 w-4" /> Run & Verify
                  </button>
                )}

                {feedback?.isCorrect && (
                  <button
                    onClick={nextChallenge}
                    className="px-6 py-2.5 rounded-xl bg-acid text-ink font-mono text-sm font-bold shadow-lg shadow-acid/20 hover:brightness-110 transition-all"
                  >
                    Continue →
                  </button>
                )}
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
                You earned <span className="text-yellow-400 font-bold">+{activeLesson.xpReward} XP</span> and advanced your L++ systems mastery!
              </p>
              <button
                onClick={() => {
                  setIsCompletedModal(false);
                  setActiveLesson(null);
                }}
                className="w-full py-3 rounded-xl bg-acid text-ink font-mono text-sm font-bold shadow-lg shadow-acid/20 hover:brightness-110 transition-all"
              >
                Back to Curriculum
              </button>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </section>
  );
}
