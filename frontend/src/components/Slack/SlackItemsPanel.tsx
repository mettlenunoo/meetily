'use client';

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { CheckCircle2, Circle, ExternalLink, Gavel, Loader2, MessageSquare, RefreshCw } from 'lucide-react';
import { slackApi, type SlackItem } from '@/lib/slack';
import { formatDue } from '@/lib/projects';

/** The bot's tasks and decisions for one project (from Slack), refreshed on request. */
export function SlackItemsPanel({ projectId }: { projectId: string }) {
  const [items, setItems] = useState<SlackItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [linked, setLinked] = useState<boolean | null>(null);

  const load = useCallback(() => {
    slackApi.items(projectId).then(setItems).catch(console.error);
  }, [projectId]);

  useEffect(() => {
    load();
    slackApi.getLink().then((s) => setLinked(s.configured)).catch(() => setLinked(false));
  }, [load]);

  const refresh = async () => {
    setLoading(true);
    try {
      const n = await slackApi.pull();
      load();
      toast.success(`Updated from Slack (${n} item${n === 1 ? '' : 's'} across projects)`);
    } catch (e) {
      toast.error(String(e));
    } finally {
      setLoading(false);
    }
  };

  if (linked === false) return null; // not connected to the bot: nothing to show

  const tasks = items.filter((i) => i.kind === 'task');
  const decisions = items.filter((i) => i.kind === 'decision');
  const synced = items[0]?.syncedAt;
  const open = (url: string) => invoke('open_external_url', { url }).catch(() => window.open(url, '_blank'));

  return (
    <section className="mb-6 rounded-lg border border-gray-200 bg-white p-4">
      <div className="mb-2 flex items-center justify-between">
        <h2 className="flex items-center gap-2 text-sm font-semibold text-gray-900">
          <MessageSquare className="w-4 h-4" /> From Slack
          {synced && (
            <span className="font-normal text-gray-500">
              as of {new Date(synced).toLocaleString(undefined, { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })}
            </span>
          )}
        </h2>
        <button type="button" onClick={refresh} disabled={loading} className="inline-flex items-center gap-1 text-xs text-gray-600 hover:text-gray-900">
          {loading ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <RefreshCw className="w-3.5 h-3.5" />} Refresh
        </button>
      </div>
      {items.length === 0 ? (
        <p className="text-sm text-gray-500">
          Tasks and decisions the bot files under this project (in its Slack channel, or from shared meetings) appear here.
        </p>
      ) : (
        <div className="space-y-3">
          {tasks.length > 0 && (
            <ul className="space-y-1.5">
              {tasks.map((t) => (
                <li key={t.key} className="flex items-start gap-2 text-sm">
                  {t.status === 'done' ? <CheckCircle2 className="mt-0.5 w-4 h-4 text-green-600" /> : <Circle className="mt-0.5 w-4 h-4 text-gray-400" />}
                  <span className={`flex-1 ${t.status === 'done' ? 'text-gray-400 line-through' : 'text-gray-900'}`}>
                    {t.title}
                    <span className="ml-2 text-xs text-gray-500">
                      {[t.assignee ?? 'no assignee', t.status === 'blocked' ? 'blocked' : null, t.dueAt ? `due ${formatDue(t.dueAt)}` : null].filter(Boolean).join(' · ')}
                      {t.overdue && <span className="ml-1 text-red-600">overdue</span>}
                    </span>
                  </span>
                </li>
              ))}
            </ul>
          )}
          {decisions.length > 0 && (
            <ul className="space-y-1.5">
              {decisions.map((d) => (
                <li key={d.key} className="flex items-start gap-2 text-sm">
                  <Gavel className="mt-0.5 w-4 h-4 text-gray-400" />
                  <span className="flex-1 text-gray-900">
                    {d.title}
                    <span className="ml-2 text-xs text-gray-500">{d.source === 'meeting' && d.meetingRef ? `from ${d.meetingRef}` : 'in Slack'}</span>
                    {d.link && (
                      <button type="button" onClick={() => open(d.link!)} className="ml-1 inline-flex items-center text-xs text-blue-600 hover:underline">
                        open <ExternalLink className="ml-0.5 w-3 h-3" />
                      </button>
                    )}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </section>
  );
}
