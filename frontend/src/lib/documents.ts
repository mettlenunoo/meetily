import { invoke } from '@tauri-apps/api/core';

export interface HearkenDocument {
  id: string;
  meetingId: string | null;
  projectId: string | null;
  templateId: string;
  title: string;
  markdown: string;
  status: 'generating' | 'ready' | 'failed';
  error: string | null;
  generatedBy: string;
  userEdited: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface DocumentTemplate {
  id: string;
  name: string;
  description: string;
}

export type DocumentOwner = { meetingId: string } | { projectId: string };

export const documentsApi = {
  list: (owner: DocumentOwner) =>
    invoke<HearkenDocument[]>('api_list_documents', {
      meetingId: 'meetingId' in owner ? owner.meetingId : null,
      projectId: 'projectId' in owner ? owner.projectId : null,
    }),
  get: (id: string) => invoke<HearkenDocument>('api_get_document', { id }),
  generate: (owner: DocumentOwner, templateId: string) =>
    invoke<HearkenDocument>('api_generate_document', {
      meetingId: 'meetingId' in owner ? owner.meetingId : null,
      projectId: 'projectId' in owner ? owner.projectId : null,
      templateId,
    }),
  update: (id: string, changes: { title?: string; markdown?: string }) =>
    invoke<HearkenDocument>('api_update_document', { id, title: changes.title ?? null, markdown: changes.markdown ?? null }),
  remove: (id: string) => invoke<void>('api_delete_document', { id }),
  templates: (forProject: boolean) => invoke<DocumentTemplate[]>('api_list_document_templates', { forProject }),
  /** Returns the saved path, or null if the user cancelled the save dialog */
  export: (id: string, format: 'md' | 'docx', html?: string) =>
    invoke<string | null>('api_export_document', { id, format, html: html ?? null }),
  print: (id: string) => invoke<void>('api_open_print_window', { id }),
};

/** "ollama:qwen2.5:14b" -> "qwen2.5:14b"; "claude-via-mcp" -> "Claude (via the connector)" */
export function describeAuthor(generatedBy: string): string {
  if (generatedBy === 'claude-via-mcp') return 'Claude (via the connector)';
  if (generatedBy === 'user') return 'you';
  if (generatedBy === 'pending') return '…';
  const i = generatedBy.indexOf(':');
  return i >= 0 ? generatedBy.slice(i + 1) : generatedBy;
}
