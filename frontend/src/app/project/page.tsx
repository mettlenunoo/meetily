'use client';

import { Suspense, useCallback, useEffect, useState } from 'react';
import { useRouter, useSearchParams } from 'next/navigation';
import { toast } from 'sonner';
import { ArrowLeft, Archive, ArchiveRestore, FolderKanban, Gavel, GraduationCap, ListTodo, Lock, MoreHorizontal, Pencil, Quote, Trash2, Video } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from '@/components/ui/dropdown-menu';
import { ConfirmationModal } from '@/components/ConfirmationModel/confirmation-modal';
import { ProjectDialog } from '@/components/Projects/ProjectDialog';
import { ActionItemRow } from '@/components/ActionItems/ActionItemRow';
import { DocumentsPanel } from '@/components/Documents/DocumentsPanel';
import { KIND_LABELS, projectsApi, type ProjectDetail, type TrackedItem } from '@/lib/projects';

function clock(seconds: number | null): string {
  if (!seconds) return '';
  const m = Math.round(seconds / 60);
  return m >= 60 ? `${Math.floor(m / 60)} h ${m % 60} min` : `${m} min`;
}

function day(iso: string): string {
  const d = new Date(iso);
  return isNaN(d.getTime()) ? iso : d.toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short', year: 'numeric' });
}

function ProjectContent() {
  const id = useSearchParams().get('id');
  const router = useRouter();
  const [detail, setDetail] = useState<ProjectDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [showDone, setShowDone] = useState(false);

  const load = useCallback(() => {
    if (!id) return;
    projectsApi.get(id).then(setDetail).catch((e) => setError(String(e)));
  }, [id]);

  useEffect(load, [load]);

  if (!id || error) {
    return <p className="p-8 text-sm text-gray-500">{error ?? 'No project selected.'}</p>;
  }
  if (!detail) {
    return <p className="p-8 text-sm text-gray-500">Loading…</p>;
  }

  const { project, meetings, actions, decisions } = detail;
  const isCourse = project.kind === 'course';
  const openActions = actions.filter((a) => a.status === 'open');
  const shownActions = showDone ? actions : openActions;

  const onChanged = (itemId: string, changes: Partial<TrackedItem>) =>
    setDetail((d) => d && { ...d, actions: d.actions.map((a) => (a.id === itemId ? { ...a, ...changes } : a)) });

  const toggleArchive = async () => {
    try {
      await projectsApi.update(project.id, { status: project.status === 'active' ? 'archived' : 'active' });
      load();
      window.dispatchEvent(new Event('projects-changed'));
    } catch (e) {
      toast.error(String(e));
    }
  };

  const remove = async () => {
    setConfirmDelete(false);
    try {
      await projectsApi.remove(project.id);
      toast.success(`Deleted ${project.name}. Its meetings are kept.`);
      window.dispatchEvent(new Event('projects-changed'));
      router.push(isCourse ? '/study' : '/projects');
    } catch (e) {
      toast.error(String(e));
    }
  };

  const Icon = isCourse ? GraduationCap : FolderKanban;

  return (
    <div className="h-screen overflow-y-auto bg-gray-50">
      <div className="mx-auto max-w-4xl px-6 py-8">
        <button
          type="button"
          onClick={() => router.push(isCourse ? '/study' : '/projects')}
          className="mb-4 inline-flex items-center gap-1 text-sm text-gray-500 hover:text-gray-800"
        >
          <ArrowLeft className="w-4 h-4" /> {isCourse ? 'Study' : 'Projects'}
        </button>

        <header className="mb-6 flex items-start justify-between gap-4">
          <div className="min-w-0">
            <h1 className="flex items-center gap-2 text-2xl font-semibold text-gray-900">
              <Icon className="w-6 h-6 flex-shrink-0" />
              <span className="truncate">{project.name}</span>
            </h1>
            <p className="mt-1 flex flex-wrap items-center gap-x-2 text-sm text-gray-500">
              <span>{KIND_LABELS[project.kind]}</span>
              {project.status === 'archived' && <span>· archived</span>}
              {project.privacy === 'local' && (
                <span className="inline-flex items-center gap-1">· <Lock className="w-3 h-3" /> local only, hidden from Claude</span>
              )}
            </p>
            {project.description && <p className="mt-2 text-sm text-gray-700">{project.description}</p>}
            {(project.people || project.keywords) && (
              <p className="mt-2 text-xs text-gray-500">
                {project.people && <>People: {project.people}</>}
                {project.people && project.keywords && ' · '}
                {project.keywords && <>Keywords: {project.keywords}</>}
              </p>
            )}
          </div>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="outline" size="icon" aria-label="Project actions">
                <MoreHorizontal className="w-4 h-4" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem onClick={() => setEditing(true)}>
                <Pencil className="w-4 h-4 mr-2" /> Edit
              </DropdownMenuItem>
              <DropdownMenuItem onClick={toggleArchive}>
                {project.status === 'active' ? <Archive className="w-4 h-4 mr-2" /> : <ArchiveRestore className="w-4 h-4 mr-2" />}
                {project.status === 'active' ? 'Archive' : 'Unarchive'}
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem onClick={() => setConfirmDelete(true)} className="text-red-600">
                <Trash2 className="w-4 h-4 mr-2" /> Delete…
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </header>

        <section className="mb-6 rounded-lg border border-gray-200 bg-white p-4">
          <div className="mb-2 flex items-center justify-between">
            <h2 className="flex items-center gap-2 text-sm font-semibold text-gray-900">
              <ListTodo className="w-4 h-4" /> Action items
              <span className="font-normal text-gray-500">{openActions.length} open</span>
            </h2>
            {actions.length > openActions.length && (
              <button type="button" onClick={() => setShowDone((s) => !s)} className="text-xs text-gray-500 hover:text-gray-800">
                {showDone ? 'Hide done' : `Show done (${actions.length - openActions.length})`}
              </button>
            )}
          </div>
          {shownActions.length === 0 ? (
            <p className="py-2 text-sm text-gray-500">No open action items.</p>
          ) : (
            <ul className="divide-y divide-gray-100">
              {shownActions.map((item) => (
                <ActionItemRow key={item.id} item={item} onChanged={onChanged} showProject={false} />
              ))}
            </ul>
          )}
        </section>

        {decisions.length > 0 && (
          <section className="mb-6 rounded-lg border border-gray-200 bg-white p-4">
            <h2 className="mb-2 flex items-center gap-2 text-sm font-semibold text-gray-900">
              <Gavel className="w-4 h-4" /> Decisions
            </h2>
            <ul className="space-y-3">
              {decisions.map((d) => (
                <li key={d.id} className="text-sm">
                  <p className="text-gray-900">{d.title}</p>
                  <p className="mt-0.5 flex flex-wrap items-center gap-1.5 text-xs text-gray-500">
                    <button type="button" onClick={() => router.push(`/meeting-details?id=${d.meetingId}`)} className="hover:underline">
                      {d.meetingTitle} · {day(d.meetingCreatedAt)}
                    </button>
                    {d.evidenceQuote && (
                      <span className="inline-flex items-center gap-1 italic">
                        <Quote className="w-3 h-3" /> “{d.evidenceQuote}”
                      </span>
                    )}
                  </p>
                </li>
              ))}
            </ul>
          </section>
        )}

        {meetings.length > 0 && (
          <DocumentsPanel owner={{ projectId: project.id }} className="mb-6 rounded-lg border border-gray-200 bg-white p-4" />
        )}

        <section className="rounded-lg border border-gray-200 bg-white p-4">
          <h2 className="mb-2 flex items-center gap-2 text-sm font-semibold text-gray-900">
            <Video className="w-4 h-4" /> {isCourse ? 'Lectures' : 'Meetings'}
            <span className="font-normal text-gray-500">{meetings.length}</span>
          </h2>
          {meetings.length === 0 ? (
            <p className="py-2 text-sm text-gray-500">
              None yet. Open a recording and use “Add to project” at the top, or accept Hearken’s suggestion.
            </p>
          ) : (
            <ul className="divide-y divide-gray-100">
              {meetings.map((m) => (
                <li key={m.id}>
                  <button
                    type="button"
                    onClick={() => router.push(`/meeting-details?id=${m.id}`)}
                    className="flex w-full items-center justify-between gap-3 py-2 text-left hover:bg-gray-50"
                  >
                    <span className="min-w-0 truncate text-sm text-gray-900">
                      {m.title}
                      {!m.isPrimary && <span className="ml-2 text-xs text-gray-400">also linked</span>}
                    </span>
                    <span className="flex-shrink-0 text-xs text-gray-500">
                      {day(m.createdAt)}{m.durationSeconds ? ` · ${clock(m.durationSeconds)}` : ''}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </section>
      </div>

      <ProjectDialog
        open={editing}
        onOpenChange={setEditing}
        project={project}
        kinds={isCourse ? ['course'] : ['work', 'client', 'personal']}
        onSaved={() => {
          load();
          window.dispatchEvent(new Event('projects-changed'));
        }}
      />
      <ConfirmationModal
        isOpen={confirmDelete}
        text={`Delete "${project.name}"? Its meetings, notes and action items are kept; they just won't be filed under it any more.`}
        onConfirm={remove}
        onCancel={() => setConfirmDelete(false)}
      />
    </div>
  );
}

export default function ProjectPage() {
  return (
    <Suspense fallback={<p className="p-8 text-sm text-gray-500">Loading…</p>}>
      <ProjectContent />
    </Suspense>
  );
}
