import { invoke } from '@tauri-apps/api/core';

export interface StudyCard {
  id: string;
  meetingId: string;
  kind: 'flashcard' | 'quiz' | 'term';
  front: string;
  back: string;
  /** quiz: JSON array of 4 options */
  choices: string | null;
  answerIndex: number | null;
  evidenceQuote: string | null;
  startSeconds: number | null;
  verified: boolean;
  source: 'ai' | 'user';
  dueOn: string | null;
  intervalDays: number;
  ease: number;
  reps: number;
  lapses: number;
  lastReviewed: string | null;
  createdAt: string;
}

export type StudyScope = { meetingId: string } | { courseId: string };

const scopeArgs = (s: StudyScope) => ({
  meetingId: 'meetingId' in s ? s.meetingId : null,
  courseId: 'courseId' in s ? s.courseId : null,
});

export const studyApi = {
  generate: (meetingId: string) =>
    invoke<{ flashcards: number; quiz: number; terms: number; unverified: number }>('api_generate_study_aids', { meetingId }),
  list: (scope: StudyScope, kind?: StudyCard['kind']) => invoke<StudyCard[]>('api_list_study_cards', { ...scopeArgs(scope), kind: kind ?? null }),
  due: (scope: StudyScope, limit = 50) => invoke<StudyCard[]>('api_due_cards', { ...scopeArgs(scope), limit }),
  review: (id: string, rating: 1 | 2 | 3 | 4) => invoke<StudyCard>('api_review_card', { id, rating }),
  add: (meetingId: string, kind: 'flashcard' | 'term', front: string, back: string) =>
    invoke<StudyCard>('api_add_study_card', { meetingId, kind, front, back }),
  remove: (id: string) => invoke<void>('api_delete_study_card', { id }),
  exportAnki: (scope: StudyScope) => invoke<string | null>('api_export_anki', scopeArgs(scope)),
};

export function scopeFromParams(params: URLSearchParams): StudyScope | null {
  const m = params.get('meeting');
  const c = params.get('course');
  return m ? { meetingId: m } : c ? { courseId: c } : null;
}

export function scopeQuery(scope: StudyScope): string {
  return 'meetingId' in scope ? `meeting=${scope.meetingId}` : `course=${scope.courseId}`;
}
