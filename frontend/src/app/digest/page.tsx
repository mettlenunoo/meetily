'use client';

import { useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { AlertCircle, CalendarRange, CheckCircle2, ChevronLeft, ChevronRight, Gavel, ListTodo, Video } from 'lucide-react';
import { intelligenceApi, type DigestItem, type WeekDigest } from '@/lib/intelligence';
import { calendarApi, type CalEvent } from '@/lib/calendar';
import { formatDue } from '@/lib/projects';

function range(d: WeekDigest): string {
  const fmt = (s: string) => {
    const [y, m, day] = s.split('-').map(Number);
    return new Date(y, m - 1, day).toLocaleDateString(undefined, { day: 'numeric', month: 'short' });
  };
  return `${fmt(d.weekStart)} – ${fmt(d.weekEnd)}`;
}

/** The weekly digest: what happened, and what needs attention. Exact, no AI. */
export default function DigestPage() {
  const router = useRouter();
  const [weeksBack, setWeeksBack] = useState(0);
  const [digest, setDigest] = useState<WeekDigest | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [today, setToday] = useState<CalEvent[]>([]);
  useEffect(() => { calendarApi.upcoming(12).then(setToday).catch(() => setToday([])); }, []);

  useEffect(() => {
    setDigest(null);
    intelligenceApi.digest(weeksBack).then(setDigest).catch((e) => setError(String(e)));
  }, [weeksBack]);

  const item = (i: DigestItem, showDue = true) => (
    <li key={i.id} className="flex flex-wrap items-baseline gap-x-2 text-sm">
      <button type="button" onClick={() => router.push(`/meeting-details?id=${i.meetingId}`)} className="text-left text-gray-900 hover:underline">
        {i.title}
      </button>
      <span className="text-xs text-gray-500">
        {[i.owner, showDue && i.dueAt ? `due ${formatDue(i.dueAt)}` : null, i.projectName ?? i.meetingTitle].filter(Boolean).join(' · ')}
      </span>
    </li>
  );

  const section = (title: string, Icon: typeof Video, items: DigestItem[], empty: string, showDue = true, tone = '') => (
    <section className="rounded-lg border border-gray-200 bg-white p-4">
      <h2 className={`mb-2 flex items-center gap-2 text-sm font-semibold ${tone || 'text-gray-900'}`}>
        <Icon className="w-4 h-4" /> {title} <span className="font-normal text-gray-500">{items.length}</span>
      </h2>
      {items.length ? <ul className="space-y-1.5">{items.map((i) => item(i, showDue))}</ul> : <p className="text-sm text-gray-500">{empty}</p>}
    </section>
  );

  return (
    <div className="h-screen overflow-y-auto bg-gray-50">
      <div className="mx-auto max-w-4xl px-6 py-8">
        <header className="mb-6 flex items-center justify-between gap-4">
          <div>
            <h1 className="flex items-center gap-2 text-2xl font-semibold text-gray-900">
              <CalendarRange className="w-6 h-6" /> Weekly digest
            </h1>
            <p className="mt-1 text-sm text-gray-500">{digest ? range(digest) : ' '}</p>
          </div>
          <div className="flex items-center gap-1">
            <button type="button" onClick={() => setWeeksBack((w) => w + 1)} className="rounded p-1.5 hover:bg-gray-200" aria-label="Previous week">
              <ChevronLeft className="w-5 h-5" />
            </button>
            <button type="button" disabled={weeksBack === 0} onClick={() => setWeeksBack((w) => Math.max(0, w - 1))} className="rounded p-1.5 hover:bg-gray-200 disabled:opacity-30" aria-label="Next week">
              <ChevronRight className="w-5 h-5" />
            </button>
          </div>
        </header>

        {error && <p className="text-sm text-red-600">{error}</p>}
        {!digest ? (
          !error && <p className="text-sm text-gray-500">Loading…</p>
        ) : (
          <div className="space-y-4">
            {digest.byProject.length > 0 && (
              <div className="grid grid-cols-2 gap-3 md:grid-cols-3">
                {digest.byProject.map((p) => (
                  <div key={p.projectName} className="rounded-lg border border-gray-200 bg-white p-3">
                    <p className="truncate text-sm font-medium text-gray-900">{p.projectName}</p>
                    <p className="mt-1 text-xs text-gray-500">
                      {p.meetings} meeting(s) · {p.decisions} decision(s)
                      {p.overdue > 0 && <span className="text-red-600"> · {p.overdue} overdue</span>}
                    </p>
                  </div>
                ))}
              </div>
            )}

            {weeksBack === 0 && today.length > 0 && (
              <section className="rounded-lg border border-gray-200 bg-white p-4">
                <h2 className="mb-2 flex items-center gap-2 text-sm font-semibold text-gray-900">
                  <CalendarRange className="w-4 h-4" /> Coming up today
                </h2>
                <ul className="space-y-1.5">
                  {today.map((e) => (
                    <li key={e.id + e.start} className="text-sm text-gray-900">
                      <span className="mr-2 font-mono text-xs text-gray-500">
                        {new Date(e.start * 1000).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })}
                      </span>
                      {e.title}
                      {e.attendees.length > 0 && <span className="ml-2 text-xs text-gray-500">with {e.attendees.slice(0, 4).join(', ')}{e.attendees.length > 4 ? '…' : ''}</span>}
                    </li>
                  ))}
                </ul>
              </section>
            )}
            {weeksBack === 0 && section('Overdue', AlertCircle, digest.overdue, 'Nothing overdue.', true, 'text-red-700')}
            {weeksBack === 0 && section('Due in the next 7 days', ListTodo, digest.dueSoon, 'Nothing due in the next week.')}

            <section className="rounded-lg border border-gray-200 bg-white p-4">
              <h2 className="mb-2 flex items-center gap-2 text-sm font-semibold text-gray-900">
                <Video className="w-4 h-4" /> Meetings <span className="font-normal text-gray-500">{digest.meetings.length}</span>
              </h2>
              {digest.meetings.length ? (
                <ul className="space-y-1.5">
                  {digest.meetings.map((m) => (
                    <li key={m.id} className="text-sm">
                      <button type="button" onClick={() => router.push(`/meeting-details?id=${m.id}`)} className="text-gray-900 hover:underline">{m.title}</button>
                      <span className="ml-2 text-xs text-gray-500">
                        {new Date(m.createdAt).toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short' })}
                        {m.projectName && ` · ${m.projectName}`}
                      </span>
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="text-sm text-gray-500">No recordings this week.</p>
              )}
            </section>

            {section('Decisions', Gavel, digest.decisions, 'No decisions recorded.', false)}
            {section('New action items', ListTodo, digest.actionsCreated, 'None.')}
            {section('Done this week', CheckCircle2, digest.actionsDone, 'Nothing ticked off yet.', false)}
          </div>
        )}
      </div>
    </div>
  );
}
