import { invoke } from '@tauri-apps/api/core';
import type { MeetingItem } from '@/components/MeetingDetails/MeetingItemsPanel';

export type ProjectKind = 'work' | 'client' | 'personal' | 'course';
export type ProjectPrivacy = 'claude' | 'local';

export interface Project {
  id: string;
  name: string;
  kind: ProjectKind;
  description: string | null;
  people: string | null;
  keywords: string | null;
  slackChannels: string | null;
  status: 'active' | 'archived';
  privacy: ProjectPrivacy;
  color: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface ProjectSummary extends Project {
  meetingCount: number;
  openActions: number;
  overdueActions: number;
  lastMeetingAt: string | null;
}

/** Create: name is required. Edit: omitted fields stay as they are, '' clears one. */
export type ProjectInput = Partial<
  Pick<Project, 'name' | 'kind' | 'description' | 'people' | 'keywords' | 'slackChannels' | 'status' | 'privacy' | 'color'>
>;

export interface TrackedItem extends MeetingItem {
  meetingTitle: string;
  meetingCreatedAt: string;
  projectId: string | null;
  projectName: string | null;
}

export interface ProjectMeeting {
  id: string;
  title: string;
  createdAt: string;
  isPrimary: boolean;
  durationSeconds: number | null;
}

export interface ProjectDetail {
  project: Project;
  meetings: ProjectMeeting[];
  actions: TrackedItem[];
  decisions: TrackedItem[];
}

export interface ProjectSuggestion {
  project: Project;
  reason: string | null;
  source: 'match' | 'ai';
}

export interface MeetingProjects {
  primary: Project | null;
  extras: Project[];
  suggestion: ProjectSuggestion | null;
}

export interface ActionItemFilter {
  status?: 'open' | 'done' | 'dismissed' | 'all';
  projectId?: string | null; // a project id, or 'none'
  owner?: string;
  /** Match the whole name (links from People) instead of part of it */
  ownerExact?: boolean;
  due?: 'any' | 'overdue' | 'today' | 'week' | 'undated';
  kind?: 'action' | 'decision' | 'all';
}

export const KIND_LABELS: Record<ProjectKind, string> = {
  work: 'Work',
  client: 'Client',
  personal: 'Personal',
  course: 'Course',
};

export const projectsApi = {
  list: (opts: { includeArchived?: boolean; kind?: ProjectKind; excludeKind?: ProjectKind } = {}) =>
    invoke<ProjectSummary[]>('api_list_projects', {
      includeArchived: opts.includeArchived ?? false,
      kind: opts.kind ?? null,
      excludeKind: opts.excludeKind ?? null,
    }),
  get: (id: string) => invoke<ProjectDetail>('api_get_project', { id }),
  create: (input: ProjectInput) => invoke<Project>('api_create_project', { input }),
  update: (id: string, input: ProjectInput) => invoke<Project>('api_update_project', { id, input }),
  remove: (id: string) => invoke<void>('api_delete_project', { id }),
  forMeeting: (meetingId: string) => invoke<MeetingProjects>('api_get_meeting_projects', { meetingId }),
  setForMeeting: (meetingId: string, primaryId: string | null, extraIds: string[]) =>
    invoke<MeetingProjects>('api_set_meeting_projects', { meetingId, primaryId, extraIds }),
  resolveSuggestion: (meetingId: string, accept: boolean) =>
    invoke<MeetingProjects>('api_resolve_project_suggestion', { meetingId, accept }),
  suggest: (meetingId: string) => invoke<MeetingProjects>('api_suggest_project', { meetingId }),
  actionItems: (filter: ActionItemFilter) => invoke<TrackedItem[]>('api_list_action_items', { filter }),
};

/** Today in local time as YYYY-MM-DD, matching how due dates are stored. */
export function todayISO(): string {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}

/** "Fri 16 Oct", with the year only when it isn't this year. */
export function formatDue(dueAt: string): string {
  const [y, m, d] = dueAt.split('-').map(Number);
  const date = new Date(y, m - 1, d);
  return date.toLocaleDateString(undefined, {
    weekday: 'short',
    day: 'numeric',
    month: 'short',
    ...(y !== new Date().getFullYear() ? { year: 'numeric' } : {}),
  });
}

export type DueState = 'overdue' | 'today' | 'soon' | 'later' | 'none';

export function dueState(item: Pick<MeetingItem, 'dueAt' | 'status'>): DueState {
  if (!item.dueAt) return 'none';
  if (item.status !== 'open') return 'later';
  const today = todayISO();
  if (item.dueAt < today) return 'overdue';
  if (item.dueAt === today) return 'today';
  const [y, m, d] = today.split('-').map(Number);
  const week = new Date(y, m - 1, d + 7);
  const weekISO = `${week.getFullYear()}-${String(week.getMonth() + 1).padStart(2, '0')}-${String(week.getDate()).padStart(2, '0')}`;
  return item.dueAt <= weekISO ? 'soon' : 'later';
}

export const DUE_STYLES: Record<DueState, string> = {
  overdue: 'border-red-200 bg-red-50 text-red-700',
  today: 'border-amber-200 bg-amber-50 text-amber-700',
  soon: 'border-purple-200 bg-purple-50 text-purple-700',
  later: 'border-gray-200 bg-gray-50 text-gray-700',
  none: 'border-dashed border-gray-300 text-gray-400',
};
