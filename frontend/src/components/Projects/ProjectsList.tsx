'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { AlertCircle, FolderKanban, GraduationCap, Lock, Plus } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { ProjectDialog } from '@/components/Projects/ProjectDialog';
import { KIND_LABELS, projectsApi, type ProjectSummary } from '@/lib/projects';

function relativeDay(iso: string | null): string {
  if (!iso) return 'No meetings yet';
  const d = new Date(iso);
  if (isNaN(d.getTime())) return '';
  const days = Math.floor((Date.now() - d.getTime()) / 86_400_000);
  if (days <= 0) return 'Last meeting today';
  if (days === 1) return 'Last meeting yesterday';
  if (days < 7) return `Last meeting ${days} days ago`;
  return `Last meeting ${d.toLocaleDateString(undefined, { day: 'numeric', month: 'short' })}`;
}

const MODES = {
  projects: {
    title: 'Projects',
    Icon: FolderKanban,
    intro: 'File meetings under projects to see their decisions and open actions in one place.',
    noun: 'project',
    emptyText:
      'Create one for each piece of work or client. Add the people and keywords that come up in its meetings and Hearken will suggest the right project after each recording.',
  },
  study: {
    title: 'Study',
    Icon: GraduationCap,
    intro: 'Your courses and their recorded lectures, kept apart from work.',
    noun: 'course',
    emptyText:
      'Create a course, then file lecture recordings under it. Lecture notes, flashcards and study guides are coming in the next phase.',
  },
} as const;

/** Projects, or (in Study mode) courses. Both are stored as projects; courses have kind 'course'. */
export function ProjectsList({ mode }: { mode: keyof typeof MODES }) {
  const { title, Icon, intro, noun, emptyText } = MODES[mode];
  const router = useRouter();
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const [showArchived, setShowArchived] = useState(false);
  const [creating, setCreating] = useState(false);
  const [loading, setLoading] = useState(true);

  const load = useCallback(() => {
    projectsApi
      .list(mode === 'study' ? { includeArchived: showArchived, kind: 'course' } : { includeArchived: showArchived, excludeKind: 'course' })
      .then(setProjects)
      .catch(console.error)
      .finally(() => setLoading(false));
  }, [showArchived, mode]);

  useEffect(load, [load]);

  return (
    <div className="h-screen overflow-y-auto bg-gray-50">
      <div className="mx-auto max-w-5xl px-6 py-8">
        <header className="mb-6 flex items-start justify-between gap-4">
          <div>
            <h1 className="flex items-center gap-2 text-2xl font-semibold text-gray-900">
              <Icon className="w-6 h-6" /> {title}
            </h1>
            <p className="mt-1 text-sm text-gray-500">{intro}</p>
          </div>
          <Button onClick={() => setCreating(true)}>
            <Plus className="w-4 h-4 mr-1" /> New {noun}
          </Button>
        </header>

        {!loading && projects.length === 0 ? (
          <div className="rounded-lg border border-dashed border-gray-300 bg-white p-10 text-center">
            <Icon className="mx-auto mb-3 w-8 h-8 text-gray-400" />
            <p className="font-medium text-gray-800">No {noun}s yet</p>
            <p className="mx-auto mt-1 max-w-md text-sm text-gray-500">{emptyText}</p>
            <Button className="mt-4" onClick={() => setCreating(true)}>
              <Plus className="w-4 h-4 mr-1" /> Create your first {noun}
            </Button>
          </div>
        ) : (
          <div className="grid grid-cols-1 gap-3 md:grid-cols-2 lg:grid-cols-3">
            {projects.map((p) => (
              <button
                key={p.id}
                type="button"
                onClick={() => router.push(`/project?id=${p.id}`)}
                className={`rounded-lg border bg-white p-4 text-left shadow-sm transition hover:border-blue-300 hover:shadow ${
                  p.status === 'archived' ? 'opacity-60' : ''
                }`}
              >
                <div className="flex items-start justify-between gap-2">
                  <p className="font-medium text-gray-900">{p.name}</p>
                  <span className="flex items-center gap-1 text-xs text-gray-500">
                    {p.privacy === 'local' && <Lock className="w-3 h-3" aria-label="Local only" />}
                    {KIND_LABELS[p.kind]}
                    {p.status === 'archived' && ' · archived'}
                  </span>
                </div>
                {p.description && <p className="mt-1 line-clamp-2 text-sm text-gray-600">{p.description}</p>}
                <div className="mt-3 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-gray-500">
                  <span>{p.meetingCount} {mode === 'study' ? 'lecture' : 'meeting'}{p.meetingCount === 1 ? '' : 's'}</span>
                  <span>{p.openActions} open action{p.openActions === 1 ? '' : 's'}</span>
                  {p.overdueActions > 0 && (
                    <span className="inline-flex items-center gap-1 text-red-600">
                      <AlertCircle className="w-3 h-3" /> {p.overdueActions} overdue
                    </span>
                  )}
                </div>
                <p className="mt-1 text-xs text-gray-400">{relativeDay(p.lastMeetingAt)}</p>
              </button>
            ))}
          </div>
        )}

        <label className="mt-6 flex items-center gap-2 text-sm text-gray-500">
          <input type="checkbox" checked={showArchived} onChange={(e) => setShowArchived(e.target.checked)} />
          Show archived {noun}s
        </label>
      </div>

      <ProjectDialog
        open={creating}
        onOpenChange={setCreating}
        kinds={mode === 'study' ? ['course'] : ['work', 'client', 'personal']}
        onSaved={(p) => router.push(`/project?id=${p.id}`)}
      />
    </div>
  );
}
