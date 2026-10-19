'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { toast } from 'sonner';
import { BookOpen, Download, GraduationCap, Layers, ListChecks, Loader2, Plus, ShieldAlert, Sparkles, Trash2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { scopeQuery, studyApi, type StudyCard, type StudyScope } from '@/lib/study';
import { clock, seekTo } from '@/lib/media';

interface StudyPanelProps {
  scope: StudyScope;
  /** Lecture pages can generate; course pages show what all lectures produced. */
  canGenerate?: boolean;
  className?: string;
}

/** Flashcards, quiz and glossary for a lecture (or a whole course), with review and Anki export. */
export function StudyPanel({ scope, canGenerate = false, className = '' }: StudyPanelProps) {
  const router = useRouter();
  const [cards, setCards] = useState<StudyCard[]>([]);
  const [due, setDue] = useState(0);
  const [generating, setGenerating] = useState(false);
  const [showTerms, setShowTerms] = useState(true);
  const [adding, setAdding] = useState<{ front: string; back: string } | null>(null);
  const key = scopeQuery(scope);

  const load = useCallback(() => {
    studyApi.list(scope).then(setCards).catch(console.error);
    studyApi.due(scope, 500).then((d) => setDue(d.length)).catch(console.error);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  useEffect(() => { load(); }, [load]);

  const generate = async () => {
    if (!('meetingId' in scope)) return;
    setGenerating(true);
    try {
      const r = await studyApi.generate(scope.meetingId);
      toast.success(`Made ${r.flashcards} flashcards, ${r.quiz} quiz questions and ${r.terms} terms`, {
        description: r.unverified ? `${r.unverified} couldn't be matched to the transcript and are marked; check them.` : 'Every one is linked to the moment in the lecture.',
      });
      load();
    } catch (e) {
      toast.error(String(e));
    } finally {
      setGenerating(false);
    }
  };

  const exportAnki = async () => {
    try {
      const path = await studyApi.exportAnki(scope);
      if (path) toast.success('Saved for Anki', { description: `In Anki: File → Import → ${path}` });
    } catch (e) {
      toast.error(String(e));
    }
  };

  const addCard = async () => {
    if (!adding || !('meetingId' in scope)) return;
    try {
      await studyApi.add(scope.meetingId, 'flashcard', adding.front, adding.back);
      setAdding(null);
      load();
    } catch (e) {
      toast.error(String(e));
    }
  };

  const flashcards = cards.filter((c) => c.kind === 'flashcard');
  const quiz = cards.filter((c) => c.kind === 'quiz');
  const terms = cards.filter((c) => c.kind === 'term').sort((a, b) => a.front.localeCompare(b.front));
  const isCourse = 'courseId' in scope;

  return (
    <section className={className} aria-label="Study">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <h3 className="flex items-center gap-2 text-sm font-semibold text-gray-900">
          <GraduationCap className="w-4 h-4" /> Study
          <span className="font-normal text-gray-500">
            {flashcards.length} flashcards · {quiz.length} quiz questions · {terms.length} terms
          </span>
        </h3>
        {canGenerate && (
          <Button size="sm" variant="outline" className="h-7" onClick={generate} disabled={generating}>
            {generating ? <Loader2 className="w-3.5 h-3.5 mr-1 animate-spin" /> : <Sparkles className="w-3.5 h-3.5 mr-1" />}
            {cards.some((c) => c.source === 'ai') ? 'Make again' : 'Make study aids'}
          </Button>
        )}
      </div>

      {cards.length === 0 ? (
        <p className="text-sm text-gray-500">
          {isCourse
            ? 'Open a lecture in this course and press “Make study aids”. Its flashcards, quiz and terms will add up here.'
            : 'Turn this lecture into flashcards, a practice quiz and a glossary. Each links to the moment it was explained.'}
          {generating && ' This can take a minute.'}
        </p>
      ) : (
        <>
          <div className="mb-3 flex flex-wrap gap-2">
            <Button size="sm" className="h-8" disabled={due === 0} onClick={() => router.push(`/study/review?${key}`)}>
              <Layers className="w-3.5 h-3.5 mr-1" /> Review {due > 0 ? `(${due} due)` : '(none due)'}
            </Button>
            <Button size="sm" variant="outline" className="h-8" disabled={quiz.length === 0} onClick={() => router.push(`/study/quiz?${key}`)}>
              <ListChecks className="w-3.5 h-3.5 mr-1" /> Practice quiz
            </Button>
            <Button size="sm" variant="outline" className="h-8" onClick={exportAnki}>
              <Download className="w-3.5 h-3.5 mr-1" /> Export to Anki
            </Button>
            {!isCourse && (
              <Button size="sm" variant="ghost" className="h-8" onClick={() => setAdding({ front: '', back: '' })}>
                <Plus className="w-3.5 h-3.5 mr-1" /> Add a flashcard
              </Button>
            )}
          </div>

          {adding && (
            <form className="mb-3 flex flex-wrap gap-2" onSubmit={(e) => { e.preventDefault(); addCard(); }}>
              <Input autoFocus value={adding.front} onChange={(e) => setAdding({ ...adding, front: e.target.value })} placeholder="Question" className="h-8 min-w-[12rem] flex-1" />
              <Input value={adding.back} onChange={(e) => setAdding({ ...adding, back: e.target.value })} placeholder="Answer" className="h-8 min-w-[12rem] flex-1" />
              <Button size="sm" type="submit" className="h-8">Add</Button>
              <Button size="sm" type="button" variant="ghost" className="h-8" onClick={() => setAdding(null)}>Cancel</Button>
            </form>
          )}

          {terms.length > 0 && (
            <div>
              <button type="button" onClick={() => setShowTerms((s) => !s)} className="mb-1 flex items-center gap-1.5 text-xs font-medium text-gray-600 hover:text-gray-900">
                <BookOpen className="w-3.5 h-3.5" /> Glossary ({terms.length}) {showTerms ? '▾' : '▸'}
              </button>
              {showTerms && (
                <dl className="divide-y divide-gray-100 rounded-md border border-gray-200">
                  {terms.map((t) => (
                    <div key={t.id} className="group flex items-start gap-3 px-3 py-2 text-sm">
                      <dt className="w-40 flex-shrink-0 font-medium text-gray-900">{t.front}</dt>
                      <dd className="flex-1 text-gray-700">
                        {t.back}
                        {t.startSeconds != null && !isCourse && (
                          <button type="button" onClick={() => seekTo(t.startSeconds!)} className="ml-2 font-mono text-xs text-gray-400 hover:text-blue-600">
                            [{clock(t.startSeconds)}]
                          </button>
                        )}
                        {!t.verified && t.source === 'ai' && (
                          <span className="ml-1 inline-flex items-center gap-0.5 text-xs text-amber-600" title="The quote for this couldn't be found in the transcript">
                            <ShieldAlert className="w-3 h-3" /> check
                          </span>
                        )}
                      </dd>
                      <button
                        type="button"
                        aria-label="Remove term"
                        onClick={() => studyApi.remove(t.id).then(load)}
                        className="opacity-0 text-gray-400 hover:text-red-600 group-hover:opacity-100"
                      >
                        <Trash2 className="w-3.5 h-3.5" />
                      </button>
                    </div>
                  ))}
                </dl>
              )}
            </div>
          )}
        </>
      )}
    </section>
  );
}
