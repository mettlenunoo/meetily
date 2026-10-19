'use client';

import { useCallback, useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { toast } from 'sonner';
import { AlertTriangle, ChevronDown, FileText, Loader2, Plus } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu';
import { DocumentViewer } from './DocumentViewer';
import { describeAuthor, documentsApi, type DocumentOwner, type DocumentTemplate, type HearkenDocument } from '@/lib/documents';

interface DocumentsPanelProps {
  owner: DocumentOwner;
  /** For project documents: courses get the study guide, other projects the status report */
  projectKind?: string;
  className?: string;
}

function when(iso: string): string {
  const d = new Date(iso);
  return isNaN(d.getTime()) ? '' : d.toLocaleString(undefined, { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' });
}

/**
 * Documents for a meeting (minutes, report, lecture notes...) or a project (status
 * report). Generated in the background with the summary model; open one to read,
 * edit or export it.
 */
export function DocumentsPanel({ owner, projectKind, className = '' }: DocumentsPanelProps) {
  const [docs, setDocs] = useState<HearkenDocument[]>([]);
  const [templates, setTemplates] = useState<DocumentTemplate[]>([]);
  const [openId, setOpenId] = useState<string | null>(null);
  const forProject = 'projectId' in owner;
  const ownerKey = 'projectId' in owner ? `p:${owner.projectId}` : `m:${owner.meetingId}`;

  const load = useCallback(() => {
    documentsApi.list(owner).then(setDocs).catch((e) => console.error('Failed to load documents:', e));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ownerKey]);

  useEffect(() => {
    load();
    documentsApi
      .templates(forProject)
      .then((ts) =>
        setTemplates(
          forProject ? ts.filter((t) => (t.id === 'course_study_guide') === (projectKind === 'course')) : ts,
        ),
      )
      .catch(console.error);
  }, [load, forProject, projectKind]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<{ id: string; status: string }>('document-updated', (e) => {
      load();
      if (e.payload.status === 'failed') toast.error('A document could not be generated');
    }).then((fn) => (unlisten = fn));
    return () => unlisten?.();
  }, [load]);

  const generate = async (t: DocumentTemplate) => {
    try {
      const doc = await documentsApi.generate(owner, t.id);
      setDocs((prev) => [doc, ...prev]);
      toast.info(`Writing ${t.name}…`, { description: 'It appears here when it is ready. You can keep working.' });
    } catch (e) {
      toast.error(String(e));
    }
  };

  return (
    <section className={className} aria-label="Documents">
      <div className="mb-3 flex items-center justify-between">
        <h3 className="flex items-center gap-2 text-sm font-semibold text-gray-900">
          <FileText className="w-4 h-4" /> Documents
        </h3>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="outline" size="sm" className="h-7">
              <Plus className="w-3.5 h-3.5 mr-1" /> Generate <ChevronDown className="w-3 h-3 ml-1" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-72">
            {templates.map((t) => (
              <DropdownMenuItem key={t.id} onClick={() => generate(t)} className="flex flex-col items-start gap-0.5">
                <span className="text-sm">{t.name}</span>
                <span className="text-xs text-gray-500">{t.description}</span>
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>

      {docs.length === 0 ? (
        <p className="text-sm text-gray-500">
          {forProject
            ? 'Generate a status report from all of this project’s meetings.'
            : 'Generate minutes, a formal report or other write-ups of this recording. The summary above stays as it is.'}
        </p>
      ) : (
        <ul className="divide-y divide-gray-100 rounded-md border border-gray-200">
          {docs.map((d) => (
            <li key={d.id}>
              <button
                type="button"
                disabled={d.status === 'generating'}
                onClick={() => setOpenId(d.id)}
                className="flex w-full items-center gap-2 px-3 py-2 text-left hover:bg-gray-50 disabled:cursor-wait"
              >
                {d.status === 'generating' ? (
                  <Loader2 className="w-4 h-4 animate-spin text-blue-500" />
                ) : d.status === 'failed' ? (
                  <AlertTriangle className="w-4 h-4 text-amber-500" />
                ) : (
                  <FileText className="w-4 h-4 text-gray-400" />
                )}
                <span className="min-w-0 flex-1 truncate text-sm text-gray-900">{d.title}</span>
                <span className="flex-shrink-0 text-xs text-gray-500">
                  {d.status === 'generating'
                    ? 'Writing…'
                    : d.status === 'failed'
                      ? 'Failed'
                      : `${d.userEdited ? 'edited' : `by ${describeAuthor(d.generatedBy)}`} · ${when(d.updatedAt)}`}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}

      <DocumentViewer
        documentId={openId}
        onClose={() => setOpenId(null)}
        onChanged={load}
      />
    </section>
  );
}
