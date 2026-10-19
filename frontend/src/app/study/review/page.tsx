'use client';

import { Suspense, useEffect, useState } from 'react';
import { useRouter, useSearchParams } from 'next/navigation';
import { ArrowLeft, CheckCircle2, PlayCircle, RotateCcw } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { scopeFromParams, studyApi, type StudyCard } from '@/lib/study';
import { clock } from '@/lib/media';

const RATINGS: { value: 1 | 2 | 3 | 4; label: string; hint: string; className: string }[] = [
  { value: 1, label: 'Again', hint: 'today', className: 'border-red-200 text-red-700 hover:bg-red-50' },
  { value: 2, label: 'Hard', hint: 'sooner', className: 'border-amber-200 text-amber-700 hover:bg-amber-50' },
  { value: 3, label: 'Good', hint: 'later', className: 'border-green-200 text-green-700 hover:bg-green-50' },
  { value: 4, label: 'Easy', hint: 'much later', className: 'border-blue-200 text-blue-700 hover:bg-blue-50' },
];

/** Flashcard review with spaced repetition: answer in your head, flip, rate yourself. */
function Review() {
  const params = useSearchParams();
  const router = useRouter();
  const scope = scopeFromParams(params);
  const [queue, setQueue] = useState<StudyCard[] | null>(null);
  const [flipped, setFlipped] = useState(false);
  const [done, setDone] = useState(0);

  useEffect(() => {
    if (scope) studyApi.due(scope, 100).then(setQueue).catch(console.error);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [params.toString()]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!queue?.length) return;
      if (e.key === ' ') { e.preventDefault(); setFlipped((f) => !f); }
      if (flipped && ['1', '2', '3', '4'].includes(e.key)) rate(Number(e.key) as 1 | 2 | 3 | 4);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  if (!scope) return <p className="p-8 text-sm text-gray-500">Nothing to review.</p>;
  if (!queue) return <p className="p-8 text-sm text-gray-500">Loading…</p>;

  const card = queue[0];
  const rate = async (rating: 1 | 2 | 3 | 4) => {
    if (!card) return;
    await studyApi.review(card.id, rating).catch(console.error);
    setFlipped(false);
    setDone((d) => d + 1);
    // "Again" puts the card back at the end of today's queue
    setQueue((q) => (q ? (rating === 1 ? [...q.slice(1), card] : q.slice(1)) : q));
  };
  const back = 'meetingId' in scope ? `/meeting-details?id=${scope.meetingId}` : `/project?id=${scope.courseId}`;

  return (
    <div className="flex h-screen flex-col bg-gray-50">
      <div className="mx-auto w-full max-w-2xl flex-1 px-6 py-8">
        <button type="button" onClick={() => router.push(back)} className="mb-6 inline-flex items-center gap-1 text-sm text-gray-500 hover:text-gray-800">
          <ArrowLeft className="w-4 h-4" /> Back
        </button>
        {!card ? (
          <div className="rounded-lg border border-gray-200 bg-white p-10 text-center">
            <CheckCircle2 className="mx-auto mb-3 w-10 h-10 text-green-600" />
            <p className="text-lg font-medium text-gray-900">All done for today</p>
            <p className="mt-1 text-sm text-gray-500">{done} card{done === 1 ? '' : 's'} reviewed. The next ones come back when they’re due.</p>
          </div>
        ) : (
          <>
            <p className="mb-2 text-sm text-gray-500">{queue.length} left · {done} done · {card.kind === 'term' ? 'Term' : 'Flashcard'}</p>
            <button
              type="button"
              onClick={() => setFlipped((f) => !f)}
              className="flex min-h-[16rem] w-full flex-col items-center justify-center rounded-xl border border-gray-200 bg-white p-8 text-center shadow-sm"
            >
              <p className="text-xl text-gray-900">{card.kind === 'term' ? `What is “${card.front}”?` : card.front}</p>
              {flipped ? (
                <>
                  <hr className="my-5 w-24 border-gray-200" />
                  <p className="text-lg text-gray-700">{card.back}</p>
                  {card.evidenceQuote && (
                    <p className="mt-4 max-w-lg text-xs italic text-gray-500">
                      “{card.evidenceQuote}”{card.startSeconds != null && ` [${clock(card.startSeconds)}]`}
                    </p>
                  )}
                </>
              ) : (
                <p className="mt-6 text-sm text-gray-400">Click or press space to show the answer</p>
              )}
            </button>
            {flipped ? (
              <div className="mt-4 grid grid-cols-4 gap-2">
                {RATINGS.map((r) => (
                  <button key={r.value} type="button" onClick={() => rate(r.value)} className={`rounded-lg border bg-white px-3 py-2 text-sm ${r.className}`}>
                    <span className="block font-medium">{r.label}</span>
                    <span className="block text-xs opacity-70">{r.value} · {r.hint}</span>
                  </button>
                ))}
              </div>
            ) : (
              <div className="mt-4 flex justify-center gap-2">
                <Button variant="outline" onClick={() => setFlipped(true)}><RotateCcw className="w-4 h-4 mr-1" /> Show answer</Button>
                {card.startSeconds != null && (
                  <Button variant="ghost" onClick={() => router.push(`/meeting-details?id=${card.meetingId}&t=${Math.floor(card.startSeconds!)}`)}>
                    <PlayCircle className="w-4 h-4 mr-1" /> Hear it explained
                  </Button>
                )}
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}

export default function ReviewPage() {
  return (
    <Suspense fallback={null}>
      <Review />
    </Suspense>
  );
}
