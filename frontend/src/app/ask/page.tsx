'use client';

import { Fragment, useEffect, useRef, useState } from 'react';
import { useRouter } from 'next/navigation';
import { Loader2, MessageCircleQuestion, Send } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Textarea } from '@/components/ui/textarea';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { intelligenceApi, type Answer, type Excerpt } from '@/lib/intelligence';
import { projectsApi, type ProjectSummary } from '@/lib/projects';
import { clock } from '@/lib/media';

interface Turn {
  question: string;
  answer?: Answer;
  error?: string;
}

const EXAMPLES = [
  'What did we decide about the rollout?',
  'What has Kofi promised to do?',
  'What did the lecturer say would be in the exam?',
];

/** Renders "[2]" citations as buttons that reveal the excerpt. */
function AnswerText({ text, excerpts, onCite }: { text: string; excerpts: Excerpt[]; onCite: (e: Excerpt) => void }) {
  const parts = text.split(/(\[\d+(?:\s*,\s*\d+)*\])/g);
  return (
    <div className="whitespace-pre-wrap text-sm leading-relaxed text-gray-900">
      {parts.map((part, i) => {
        const m = part.match(/^\[(\d+(?:\s*,\s*\d+)*)\]$/);
        if (!m) return <Fragment key={i}>{part}</Fragment>;
        return (
          <Fragment key={i}>
            {m[1].split(',').map((n) => {
              const e = excerpts.find((x) => x.n === Number(n.trim()));
              return e ? (
                <button
                  key={n}
                  type="button"
                  onClick={() => onCite(e)}
                  className="mx-0.5 rounded bg-blue-50 px-1 text-xs font-medium text-blue-700 hover:bg-blue-100"
                  title={`${e.meetingTitle}, ${e.meetingDate}`}
                >
                  {e.n}
                </button>
              ) : null;
            })}
          </Fragment>
        );
      })}
    </div>
  );
}

/** Ask a question across all your meetings; answers cite the passages they came from. */
export default function AskPage() {
  const router = useRouter();
  const [turns, setTurns] = useState<Turn[]>([]);
  const [question, setQuestion] = useState('');
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const [scope, setScope] = useState('all');
  const [asking, setAsking] = useState(false);
  const [openExcerpt, setOpenExcerpt] = useState<Excerpt | null>(null);
  const bottom = useRef<HTMLDivElement>(null);

  useEffect(() => { projectsApi.list({ includeArchived: true }).then(setProjects).catch(console.error); }, []);
  useEffect(() => { bottom.current?.scrollIntoView({ behavior: 'smooth' }); }, [turns]);

  const ask = async (q: string) => {
    const text = q.trim();
    if (!text || asking) return;
    setQuestion('');
    setAsking(true);
    setTurns((t) => [...t, { question: text }]);
    try {
      const answer = await intelligenceApi.ask(text, scope === 'all' ? null : scope);
      setTurns((t) => t.map((turn, i) => (i === t.length - 1 ? { ...turn, answer } : turn)));
    } catch (e) {
      setTurns((t) => t.map((turn, i) => (i === t.length - 1 ? { ...turn, error: String(e) } : turn)));
    } finally {
      setAsking(false);
    }
  };

  const openMeeting = (e: Excerpt) =>
    router.push(`/meeting-details?id=${e.meetingId}${e.startSeconds != null ? `&t=${Math.floor(e.startSeconds)}` : ''}`);

  return (
    <div className="flex h-screen flex-col bg-gray-50">
      <div className="flex-1 overflow-y-auto">
        <div className="mx-auto max-w-3xl px-6 py-8">
          <h1 className="flex items-center gap-2 text-2xl font-semibold text-gray-900">
            <MessageCircleQuestion className="w-6 h-6" /> Ask Hearken
          </h1>
          <p className="mt-1 text-sm text-gray-500">
            Answers come only from your recordings, notes and documents, with numbered sources. If it isn’t in there,
            Hearken says so rather than guessing.
          </p>

          {turns.length === 0 && (
            <div className="mt-6 flex flex-wrap gap-2">
              {EXAMPLES.map((e) => (
                <button key={e} type="button" onClick={() => ask(e)} className="rounded-full border border-gray-200 bg-white px-3 py-1.5 text-sm text-gray-700 hover:bg-gray-50">
                  {e}
                </button>
              ))}
            </div>
          )}

          <div className="mt-6 space-y-6">
            {turns.map((turn, i) => (
              <div key={i} className="space-y-2">
                <p className="ml-auto w-fit max-w-[85%] rounded-lg bg-blue-600 px-3 py-2 text-sm text-white">{turn.question}</p>
                <div className="rounded-lg border border-gray-200 bg-white p-4">
                  {turn.error ? (
                    <p className="text-sm text-red-600">{turn.error}</p>
                  ) : !turn.answer ? (
                    <p className="flex items-center gap-2 text-sm text-gray-500"><Loader2 className="w-4 h-4 animate-spin" /> Looking through your meetings…</p>
                  ) : (
                    <>
                      {turn.answer.found ? (
                        <AnswerText text={turn.answer.answer} excerpts={turn.answer.excerpts} onCite={setOpenExcerpt} />
                      ) : (
                        <p className="text-sm text-gray-700">{turn.answer.answer}</p>
                      )}
                      {turn.answer.excerpts.length > 0 && (
                        <details className="mt-3 text-xs text-gray-500" open={turn.answer.found}>
                          <summary className="cursor-pointer">
                            {turn.answer.found ? 'Sources' : 'Closest passages found'} · answered by {turn.answer.model}
                            {!turn.answer.usedMeaningSearch && ' · keyword search only'}
                          </summary>
                          <ul className="mt-2 space-y-1.5">
                            {turn.answer.excerpts
                              .filter((e) => !turn.answer!.found || e.cited)
                              .map((e) => (
                                <li key={e.n}>
                                  <button type="button" onClick={() => openMeeting(e)} className="text-left hover:text-gray-900">
                                    <span className="font-medium text-gray-700">[{e.n}] {e.meetingTitle}</span>, {e.meetingDate}
                                    {e.startSeconds != null && ` at ${clock(e.startSeconds)}`} · {e.source}
                                    <span className="block line-clamp-2 italic">“{e.text}”</span>
                                  </button>
                                </li>
                              ))}
                          </ul>
                        </details>
                      )}
                    </>
                  )}
                </div>
              </div>
            ))}
          </div>
          <div ref={bottom} />
        </div>
      </div>

      {openExcerpt && (
        <div className="border-t border-gray-200 bg-white px-6 py-3 text-sm">
          <div className="mx-auto flex max-w-3xl items-start gap-3">
            <div className="flex-1">
              <p className="text-xs text-gray-500">
                [{openExcerpt.n}] {openExcerpt.meetingTitle}, {openExcerpt.meetingDate}
                {openExcerpt.startSeconds != null && ` at ${clock(openExcerpt.startSeconds)}`}
              </p>
              <p className="mt-1 italic text-gray-800">“{openExcerpt.text}”</p>
            </div>
            <Button size="sm" variant="outline" onClick={() => openMeeting(openExcerpt)}>Open</Button>
            <button type="button" onClick={() => setOpenExcerpt(null)} className="text-xs text-gray-500 hover:text-gray-800">Close</button>
          </div>
        </div>
      )}

      <form
        className="border-t border-gray-200 bg-white px-6 py-3"
        onSubmit={(e) => { e.preventDefault(); ask(question); }}
      >
        <div className="mx-auto flex max-w-3xl items-end gap-2">
          <Select value={scope} onValueChange={setScope}>
            <SelectTrigger className="w-44"><SelectValue /></SelectTrigger>
            <SelectContent>
              <SelectItem value="all">All meetings</SelectItem>
              {projects.map((p) => <SelectItem key={p.id} value={p.id}>{p.name}</SelectItem>)}
            </SelectContent>
          </Select>
          <Textarea
            value={question}
            onChange={(e) => setQuestion(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); ask(question); } }}
            placeholder="Ask about your meetings…"
            rows={1}
            className="min-h-9 flex-1 resize-none"
          />
          <Button type="submit" disabled={!question.trim() || asking}>
            {asking ? <Loader2 className="w-4 h-4 animate-spin" /> : <Send className="w-4 h-4" />}
          </Button>
        </div>
      </form>
    </div>
  );
}
