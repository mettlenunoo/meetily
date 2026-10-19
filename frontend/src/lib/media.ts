import { invoke } from '@tauri-apps/api/core';

export type RecordingType = 'meeting' | 'lecture' | 'interview' | 'voice_note';

export const RECORDING_TYPE_LABELS: Record<RecordingType, string> = {
  meeting: 'Meeting',
  lecture: 'Lecture',
  interview: 'Interview',
  voice_note: 'Voice note',
};

export interface AudioTrack {
  /** mixed = everyone, mic = you, system = everyone else */
  kind: 'mixed' | 'mic' | 'system';
  path: string;
}

export interface Bookmark {
  id: string;
  meetingId: string;
  seconds: number;
  note: string | null;
  createdAt: string;
}

export interface MeetingMedia {
  recordingType: RecordingType;
  defaultTemplate: string;
  tracks: AudioTrack[];
  bookmarks: Bookmark[];
  speakersLabeled: boolean;
}

export const mediaApi = {
  get: (meetingId: string) => invoke<MeetingMedia>('api_get_meeting_media', { meetingId }),
  setType: (meetingId: string, recordingType: RecordingType) =>
    invoke<void>('api_set_recording_type', { meetingId, recordingType }),
  addBookmark: (meetingId: string, seconds: number, note?: string) =>
    invoke<Bookmark>('api_add_bookmark', { meetingId, seconds, note: note ?? null }),
  deleteBookmark: (id: string) => invoke<void>('api_delete_bookmark', { id }),
  labelSpeakers: (meetingId: string) => invoke<{ me: number; others: number; unclear: number }>('api_label_speakers', { meetingId }),
  /** Marks "now" in the recording in progress */
  addLiveBookmark: (note?: string) => invoke<{ seconds: number; note: string | null }>('api_add_live_bookmark', { note: note ?? null }),
};

const SEEK_EVENT = 'hearken-seek';

/** Ask the meeting's audio player (if one is open) to jump to a moment and play. */
export function seekTo(seconds: number) {
  window.dispatchEvent(new CustomEvent<number>(SEEK_EVENT, { detail: seconds }));
}

export function onSeek(handler: (seconds: number) => void): () => void {
  const listener = (e: Event) => handler((e as CustomEvent<number>).detail);
  window.addEventListener(SEEK_EVENT, listener);
  return () => window.removeEventListener(SEEK_EVENT, listener);
}

export function clock(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  const h = Math.floor(total / 3600);
  const mm = String(Math.floor((total % 3600) / 60)).padStart(2, '0');
  const ss = String(total % 60).padStart(2, '0');
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}
