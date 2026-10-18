'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { useRouter } from 'next/navigation';
import { toast } from 'sonner';
import { Check, ChevronDown, FolderKanban, GraduationCap, Lightbulb, Lock, Plus, X } from 'lucide-react';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { Button } from '@/components/ui/button';
import { ProjectDialog } from './ProjectDialog';
import { projectsApi, type MeetingProjects, type Project, type ProjectSummary } from '@/lib/projects';

interface MeetingProjectBarProps {
  meetingId: string;
}

const EMPTY: MeetingProjects = { primary: null, extras: [], suggestion: null };

function ProjectIcon({ project, className = 'w-3.5 h-3.5' }: { project: Pick<Project, 'kind' | 'privacy'>; className?: string }) {
  const Icon = project.kind === 'course' ? GraduationCap : FolderKanban;
  return (
    <>
      <Icon className={className} />
      {project.privacy === 'local' && <Lock className="w-3 h-3 text-gray-400" aria-label="Local only" />}
    </>
  );
}

/**
 * Which project (or course) a meeting belongs to: one main project plus optional extras.
 * When the meeting has none, Hearken may suggest one with its reason; nothing is filed
 * until the user accepts.
 */
export function MeetingProjectBar({ meetingId }: MeetingProjectBarProps) {
  const router = useRouter();
  const [state, setState] = useState<MeetingProjects>(EMPTY);
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [creating, setCreating] = useState(false);
  const askedFor = useRef<string | null>(null);

  const load = useCallback(async () => {
    try {
      let current = await projectsApi.forMeeting(meetingId);
      // Ask once per meeting view for a quick (no-model) suggestion.
      if (!current.primary && !current.suggestion && askedFor.current !== meetingId) {
        askedFor.current = meetingId;
        current = await projectsApi.suggest(meetingId);
      }
      setState(current);
    } catch (error) {
      console.error('Failed to load meeting projects:', error);
    }
  }, [meetingId]);

  const loadProjects = useCallback(() => {
    projectsApi.list().then(setProjects).catch((e) => console.error('Failed to load projects:', e));
  }, []);

  useEffect(() => {
    setState(EMPTY);
    load();
    loadProjects();
  }, [load, loadProjects]);

  const apply = async (primaryId: string | null, extraIds: string[]) => {
    try {
      setState(await projectsApi.setForMeeting(meetingId, primaryId, extraIds));
      window.dispatchEvent(new Event('projects-changed'));
    } catch (error) {
      toast.error(`Could not update projects: ${error}`);
    }
  };

  const extraIds = state.extras.map((p) => p.id);
  const setMain = (id: string) => apply(id, extraIds.filter((e) => e !== id).concat(state.primary && state.primary.id !== id ? [state.primary.id] : []));
  const toggleExtra = (id: string) =>
    apply(state.primary?.id ?? null, extraIds.includes(id) ? extraIds.filter((e) => e !== id) : [...extraIds, id]);
  const removeMain = () => apply(null, extraIds);

  const resolve = async (accept: boolean) => {
    try {
      setState(await projectsApi.resolveSuggestion(meetingId, accept));
      if (accept) window.dispatchEvent(new Event('projects-changed'));
    } catch (error) {
      toast.error(String(error));
    }
  };

  const picker = (
    <Popover open={pickerOpen} onOpenChange={(o) => { setPickerOpen(o); if (o) loadProjects(); }}>
      <PopoverTrigger asChild>
        {state.primary ? (
          <button type="button" className="inline-flex items-center gap-1 text-xs text-gray-500 hover:text-gray-800">
            Change <ChevronDown className="w-3 h-3" />
          </button>
        ) : (
          <button
            type="button"
            className="inline-flex items-center gap-1.5 rounded-md border border-dashed border-gray-300 px-2 py-1 text-xs text-gray-600 hover:border-gray-400 hover:text-gray-900"
          >
            <FolderKanban className="w-3.5 h-3.5" /> Add to project
          </button>
        )}
      </PopoverTrigger>
      <PopoverContent className="w-80 p-2" align="start">
        <p className="px-2 pb-1 text-xs text-gray-500">Main project, plus any others it also belongs to</p>
        <div className="max-h-72 overflow-y-auto">
          {projects.length === 0 && <p className="px-2 py-3 text-sm text-gray-500">No projects yet.</p>}
          {projects.map((p) => {
            const isMain = state.primary?.id === p.id;
            const isExtra = extraIds.includes(p.id);
            return (
              <div key={p.id} className="flex items-center gap-2 rounded px-2 py-1.5 hover:bg-gray-50">
                <span className="flex flex-1 min-w-0 items-center gap-1.5 text-sm text-gray-800">
                  <ProjectIcon project={p} />
                  <span className="truncate">{p.name}</span>
                </span>
                <button
                  type="button"
                  onClick={() => (isMain ? removeMain() : setMain(p.id))}
                  className={`rounded px-1.5 py-0.5 text-xs ${isMain ? 'bg-blue-600 text-white' : 'text-gray-500 hover:bg-blue-50 hover:text-blue-700'}`}
                >
                  {isMain ? 'Main ✓' : 'Main'}
                </button>
                {!isMain && (
                  <button
                    type="button"
                    onClick={() => toggleExtra(p.id)}
                    className={`rounded px-1.5 py-0.5 text-xs ${isExtra ? 'bg-gray-700 text-white' : 'text-gray-500 hover:bg-gray-100'}`}
                  >
                    {isExtra ? 'Also ✓' : 'Also'}
                  </button>
                )}
              </div>
            );
          })}
        </div>
        <button
          type="button"
          onClick={() => { setPickerOpen(false); setCreating(true); }}
          className="mt-1 flex w-full items-center gap-1.5 rounded px-2 py-1.5 text-sm text-blue-700 hover:bg-blue-50"
        >
          <Plus className="w-3.5 h-3.5" /> New project…
        </button>
      </PopoverContent>
    </Popover>
  );

  return (
    <div className="border-b border-gray-200 bg-white px-4 py-2 space-y-2">
      <div className="flex flex-wrap items-center gap-2 text-sm">
        {state.primary && (
          <button
            type="button"
            onClick={() => router.push(`/project?id=${state.primary!.id}`)}
            className="inline-flex items-center gap-1.5 rounded-md bg-blue-50 px-2 py-1 text-xs font-medium text-blue-800 hover:bg-blue-100"
            title="Open project"
          >
            <ProjectIcon project={state.primary} />
            {state.primary.name}
          </button>
        )}
        {state.extras.map((p) => (
          <span key={p.id} className="inline-flex items-center gap-1 rounded-md bg-gray-100 px-2 py-1 text-xs text-gray-700">
            <button type="button" onClick={() => router.push(`/project?id=${p.id}`)} className="inline-flex items-center gap-1 hover:underline">
              <ProjectIcon project={p} className="w-3 h-3" />
              {p.name}
            </button>
            <button type="button" aria-label={`Remove ${p.name}`} onClick={() => toggleExtra(p.id)} className="text-gray-400 hover:text-red-600">
              <X className="w-3 h-3" />
            </button>
          </span>
        ))}
        {picker}
      </div>

      {state.suggestion && (
        <div className="flex flex-wrap items-center gap-2 rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-900">
          <Lightbulb className="w-4 h-4 flex-shrink-0" />
          <span className="flex-1 min-w-[12rem]">
            Looks like <span className="font-semibold">{state.suggestion.project.name}</span>
            {state.suggestion.reason ? <span className="text-amber-800">: {state.suggestion.reason}</span> : null}
            {state.suggestion.source === 'ai' && <span className="text-amber-700"> (suggested by the summary model)</span>}
          </span>
          <Button size="sm" className="h-7" onClick={() => resolve(true)}>
            <Check className="w-3.5 h-3.5 mr-1" /> File it there
          </Button>
          <Button size="sm" variant="outline" className="h-7 bg-white" onClick={() => setPickerOpen(true)}>
            Choose another
          </Button>
          <button type="button" onClick={() => resolve(false)} className="text-amber-800 hover:underline">
            Not this
          </button>
        </div>
      )}

      <ProjectDialog
        open={creating}
        onOpenChange={setCreating}
        kinds={['work', 'client', 'personal', 'course']}
        onSaved={(p) => {
          loadProjects();
          setMain(p.id);
        }}
      />
    </div>
  );
}
