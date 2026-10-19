'use client';

import { useEffect, useState } from 'react';
import { toast } from 'sonner';
import { Loader2, RefreshCw, Sparkles } from 'lucide-react';
import { DocumentMarkdown } from '@/components/Documents/DocumentMarkdown';
import { intelligenceApi } from '@/lib/intelligence';

/** "Where things stand": a short running brief, refreshed after each summary in the project. */
export function ProjectBrief({ projectId, hasMeetings }: { projectId: string; hasMeetings: boolean }) {
  const [brief, setBrief] = useState<{ brief: string | null; updatedAt: string | null } | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    intelligenceApi.brief(projectId).then(setBrief).catch(console.error);
  }, [projectId]);

  if (!hasMeetings) return null;

  const refresh = async () => {
    setBusy(true);
    try {
      setBrief(await intelligenceApi.refreshBrief(projectId));
    } catch (e) {
      toast.error(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="mb-6 rounded-lg border border-blue-100 bg-blue-50/40 p-4">
      <div className="mb-2 flex items-center justify-between">
        <h2 className="flex items-center gap-2 text-sm font-semibold text-gray-900">
          <Sparkles className="w-4 h-4 text-blue-600" /> Where things stand
          {brief?.updatedAt && (
            <span className="font-normal text-gray-500">
              {new Date(brief.updatedAt).toLocaleString(undefined, { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })}
            </span>
          )}
        </h2>
        <button type="button" onClick={refresh} disabled={busy} className="inline-flex items-center gap-1 text-xs text-gray-600 hover:text-gray-900">
          {busy ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <RefreshCw className="w-3.5 h-3.5" />} {brief?.brief ? 'Update' : 'Write brief'}
        </button>
      </div>
      {brief?.brief ? (
        <DocumentMarkdown markdown={brief.brief} />
      ) : (
        <p className="text-sm text-gray-500">
          A short status note from this project’s meetings and open actions. It updates itself after each new summary.
        </p>
      )}
    </section>
  );
}
