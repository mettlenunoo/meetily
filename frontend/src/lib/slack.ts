import { invoke } from '@tauri-apps/api/core';

export interface BotLink {
  mode: 'local' | 'ssh';
  python: string;
  script: string;
  sshHost: string | null;
}

export interface SlackShare {
  packetId: string;
  meetingId: string;
  channel: string;
  ts: string | null;
  permalink: string | null;
  tasksProposed: number;
  decisionsSaved: number;
  sharedAt: string;
}

export interface SlackItem {
  key: string;
  projectId: string;
  kind: 'task' | 'decision';
  title: string;
  status: 'todo' | 'in_progress' | 'blocked' | 'done' | null;
  assignee: string | null;
  dueAt: string | null;
  overdue: boolean;
  link: string | null;
  source: 'slack' | 'meeting' | null;
  meetingRef: string | null;
  updatedAt: string | null;
  syncedAt: string;
}

/** What "Send to Slack" will send: never the transcript or audio. */
export interface SharePacket {
  packet_id: string;
  meeting: { id: string; title: string; date: string };
  project: { id: string; name: string; slack_channels: string[] } | null;
  summary_markdown: string | null;
  action_items: { title: string; owner: string | null; due_at: string | null; due_words: string | null; quote: string | null; at: string | null }[];
  decisions: { title: string; at: string | null }[];
}

export const slackApi = {
  getLink: () => invoke<{ link: BotLink; configured: boolean }>('api_get_bot_link'),
  saveLink: (link: BotLink) => invoke<void>('api_save_bot_link', { link }),
  test: (link?: BotLink) => invoke<{ ok: boolean; slack: boolean; projects: number }>('api_test_bot_link', { link: link ?? null }),
  syncProjects: () => invoke<{ projects: number; unresolved_channels: string[] }>('api_sync_bot_projects'),
  preview: (meetingId: string) => invoke<SharePacket>('api_preview_meeting_share', { meetingId }),
  shares: (meetingId: string) => invoke<SlackShare[]>('api_list_meeting_shares', { meetingId }),
  share: (meetingId: string, channel: string) => invoke<SlackShare>('api_share_meeting', { meetingId, channel }),
  pull: () => invoke<number>('api_pull_from_bot'),
  items: (projectId: string) => invoke<SlackItem[]>('api_list_slack_items', { projectId }),
};
