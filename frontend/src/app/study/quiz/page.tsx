'use client';

import { Suspense, useEffect, useMemo, useState } from 'react';
import { useRouter, useSearchParams } from 'next/navigation';
import { ArrowLeft, CheckCircle2, PlayCircle, XCircle } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { scopeFromParams, studyApi, type StudyCard } from '@/lib/study';

function shuffle<T>(items: T[]): T[] {
  const a = [...items];
  for (let i = a.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [a[i], a[j]] = [a[j], a[i]];
  }
  return a;
}

/** A practice quiz from the lecture(s); wrong answers link to the moment it was explained. */
function Quiz() {
  const params = useSearchParams();
  const router = useRouter();
  const scope = scopeFromParams(params);
  const [questions, setQuestions] = useState<StudyCard[] | null>(null);
  const [index, setIndex] = useState(0);
  const [picked, setPicked] = useState<number | null>(null);
  const [score, setScore] = useState(0);

  useEffect(() => {
    if (scope) studyApi.list(scope, 'quiz').then((q) => setQuestions(shuffle(q).slice(0, 15))).catch(console.error);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [params.toString()]);

  const q = questions?.[index];
  const choices: string[] = useMemo(() => {
    try { return q?.choices ? JSON.parse(q.choices) : []; } catch { return []; }
  }, [q]);

  if (!scope) return <p className="p-8 text-sm text-gray-500">No quiz selected.</p>;
  if (!questions) return <p className="p-8 text-sm text-gray-500">Loading…</p>;
  const back = 'meetingId' in scope ? `/meeting-details?id=${scope.meetingId}` : `/project?id=${scope.courseId}`;

  const pick = (i: number) => {
    if (picked !== null || !q) return;
    setPicked(i);
    if (i === q.answerIndex) setScore((s) => s + 1);
  };
  const next = () => { setPicked(null); setIndex((i) => i + 1); };

  return (
    <div className="h-screen overflow-y-auto bg-gray-50">
      <div className="mx-auto max-w-2xl px-6 py-8">
        <button type="button" onClick={() => router.push(back)} className="mb-6 inline-flex items-center gap-1 text-sm text-gray-500 hover:text-gray-800">
          <ArrowLeft className="w-4 h-4" /> Back
        </button>
        {!q ? (
          <div className="rounded-lg border border-gray-200 bg-white p-10 text-center">
            <p className="text-3xl font-semibold text-gray-900">{score} / {questions.length}</p>
            <p className="mt-2 text-sm text-gray-500">
              {questions.length === 0 ? 'No quiz questions yet. Make study aids from a lecture first.' : score === questions.length ? 'All correct.' : 'Go over the ones you missed, then try again.'}
            </p>
            {questions.length > 0 && (
              <Button className="mt-4" onClick={() => { setQuestions(shuffle(questions)); setIndex(0); setScore(0); }}>Try again</Button>
            )}
          </div>
        ) : (
          <div className="rounded-lg border border-gray-200 bg-white p-6">
            <p className="text-xs text-gray-500">Question {index + 1} of {questions.length} · score {score}</p>
            <p className="mt-2 text-lg text-gray-900">{q.front}</p>
            <div className="mt-4 space-y-2">
              {choices.map((c, i) => {
                const right = picked !== null && i === q.answerIndex;
                const wrong = picked === i && i !== q.answerIndex;
                return (
                  <button
                    key={i}
                    type="button"
                    onClick={() => pick(i)}
                    disabled={picked !== null}
                    className={`flex w-full items-center gap-2 rounded-md border px-3 py-2 text-left text-sm ${
                      right ? 'border-green-300 bg-green-50 text-green-900' : wrong ? 'border-red-300 bg-red-50 text-red-900' : 'border-gray-200 hover:bg-gray-50'
                    }`}
                  >
                    {right ? <CheckCircle2 className="w-4 h-4 text-green-600" /> : wrong ? <XCircle className="w-4 h-4 text-red-600" /> : <span className="w-4 text-center text-xs text-gray-400">{'ABCD'[i]}</span>}
                    {c}
                  </button>
                );
              })}
            </div>
            {picked !== null && (
              <div className="mt-4 space-y-3">
                <p className="text-sm text-gray-700">{q.back}</p>
                <div className="flex gap-2">
                  <Button onClick={next}>{index + 1 < questions.length ? 'Next question' : 'See score'}</Button>
                  {q.startSeconds != null && (
                    <Button variant="ghost" onClick={() => router.push(`/meeting-details?id=${q.meetingId}&t=${Math.floor(q.startSeconds!)}`)}>
                      <PlayCircle className="w-4 h-4 mr-1" /> Hear it explained
                    </Button>
                  )}
                </div>
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

export default function QuizPage() {
  return (
    <Suspense fallback={null}>
      <Quiz />
    </Suspense>
  );
}
