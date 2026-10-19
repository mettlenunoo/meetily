import { invoke } from '@tauri-apps/api/core';

export interface IntelligenceStatus {
  semanticSearch: boolean;
  embedModel: string;
  modelInstalled: boolean;
  indexedMeetings: number;
  totalMeetings: number;
  passages: number;
}

export interface MeaningHit {
  meetingId: string;
  meetingTitle: string;
  meetingDate: string;
  source: string;
  startSeconds: number | null;
  text: string;
  score: number;
}

export interface Excerpt {
  n: number;
  meetingId: string;
  meetingTitle: string;
  meetingDate: string;
  source: string;
  startSeconds: number | null;
  text: string;
  cited: boolean;
}

export interface Answer {
  found: boolean;
  answer: string;
  excerpts: Excerpt[];
  model: string;
  usedMeaningSearch: boolean;
}

export interface DigestItem {
  id: string;
  meetingId: string;
  meetingTitle: string;
  title: string;
  owner: string | null;
  dueAt: string | null;
  status: string;
  projectName: string | null;
}

export interface WeekDigest {
  weekStart: string;
  weekEnd: string;
  meetings: { id: string; title: string; createdAt: string; projectName: string | null }[];
  decisions: DigestItem[];
  actionsCreated: DigestItem[];
  actionsDone: DigestItem[];
  overdue: DigestItem[];
  dueSoon: DigestItem[];
  byProject: { projectName: string; meetings: number; decisions: number; openActions: number; overdue: number }[];
}

export const intelligenceApi = {
  status: () => invoke<IntelligenceStatus>('api_get_intelligence_status'),
  setSemanticSearch: (enabled: boolean) => invoke<void>('api_set_semantic_search', { enabled }),
  reindex: () => invoke<[number, number]>('api_reindex_now'),
  downloadModel: (modelName: string) => invoke<void>('pull_ollama_model', { modelName, endpoint: null }),
  search: (query: string, limit = 8) => invoke<MeaningHit[]>('api_semantic_search', { query, limit }),
  ask: (question: string, projectId?: string | null) => invoke<Answer>('api_ask', { question, projectId: projectId ?? null }),
  brief: (projectId: string) => invoke<{ brief: string | null; updatedAt: string | null }>('api_get_project_brief', { projectId }),
  refreshBrief: (projectId: string) => invoke<{ brief: string | null; updatedAt: string | null }>('api_refresh_project_brief', { projectId }),
  digest: (weeksBack = 0) => invoke<WeekDigest>('api_weekly_digest', { weeksBack }),
};
