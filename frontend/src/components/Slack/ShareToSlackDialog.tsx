'use client';

import { useEffect, useState } from 'react';
import { toast } from 'sonner';
import { ExternalLink, Gavel, ListTodo, Loader2, Send, ShieldCheck } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { Dialog, DialogContent, DialogFooter, DialogTitle } from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { DocumentMarkdown } from '@/components/Documents/DocumentMarkdown';
import { slackApi, type SharePacket, type SlackShare } from '@/lib/slack';

interface ShareToSlackDialogProps {
  meetingId: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

/** Shows exactly what will be posted, then shares the meeting through Desmond's bot. */
export function ShareToSlackDialog({ meetingId, open, onOpenChange }: ShareToSlackDialogProps) {
  const [packet, setPacket] = useState<SharePacket | null>(null);
  const [shares, setShares] = useState<SlackShare[]>([]);
  const [channel, setChannel] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);

  useEffect(() => {
    if (!open) return;
    setError(null);
    setPacket(null);
    slackApi
      .preview(meetingId)
      .then((p) => {
        setPacket(p);
        setChannel((c) => c || p.project?.slack_channels[0] || '');
      })
      .catch((e) => setError(String(e)));
    slackApi.shares(meetingId).then(setShares).catch(() => setShares([]));
  }, [open, meetingId]);

  const alreadySent = packet && shares.some((s) => s.packetId === packet.packet_id);

  const send = async () => {
    setSending(true);
    try {
      const share = await slackApi.share(meetingId, channel);
      setShares((prev) => [share, ...prev.filter((s) => s.packetId !== share.packetId)]);
      toast.success(`Shared to ${channel}`, {
        description: share.tasksProposed
          ? `${share.tasksProposed} action item(s) are waiting in the thread for someone to press “Create selected”.`
          : undefined,
      });
      onOpenChange(false);
    } catch (e) {
      toast.error(String(e));
    } finally {
      setSending(false);
    }
  };

  const openLink = (url: string) => invoke('open_external_url', { url }).catch(() => window.open(url, '_blank'));

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[90vh] max-w-2xl flex-col">
        <DialogTitle>Send to Slack</DialogTitle>
        {error ? (
          <p className="rounded-md border border-amber-200 bg-amber-50 p-3 text-sm text-amber-900">{error}</p>
        ) : !packet ? (
          <p className="text-sm text-gray-500">Preparing…</p>
        ) : (
          <div className="min-h-0 flex-1 space-y-4 overflow-y-auto pr-1">
            <p className="flex items-start gap-2 rounded-md bg-green-50 p-3 text-xs text-green-900">
              <ShieldCheck className="mt-0.5 w-4 h-4 flex-shrink-0" />
              <span>
                This is everything that will be sent: the summary, the open action items and the decisions. The transcript and
                audio stay on this Mac. Action items appear as a preview in the thread, and become tasks only when someone
                presses “Create selected”.
              </span>
            </p>
            <div className="space-y-1.5">
              <Label htmlFor="slack-channel">Channel</Label>
              <Input id="slack-channel" value={channel} onChange={(e) => setChannel(e.target.value)} placeholder="#project-channel" />
              {packet.project && <p className="text-xs text-gray-500">Project: {packet.project.name}</p>}
            </div>

            <div className="rounded-md border border-gray-200 p-3">
              <p className="mb-2 text-sm font-medium text-gray-900">
                {packet.meeting.title} · {packet.meeting.date}
              </p>
              {packet.summary_markdown ? (
                <DocumentMarkdown markdown={packet.summary_markdown} className="max-h-60 overflow-y-auto text-sm" />
              ) : (
                <p className="text-sm text-gray-500">No summary yet; only the items below will be sent.</p>
              )}
            </div>

            {packet.decisions.length > 0 && (
              <div>
                <p className="mb-1 flex items-center gap-1.5 text-sm font-medium text-gray-900"><Gavel className="w-4 h-4" /> Decisions → decision log</p>
                <ul className="list-disc pl-6 text-sm text-gray-700">
                  {packet.decisions.map((d, i) => <li key={i}>{d.title}{d.at && <span className="text-gray-400"> [{d.at}]</span>}</li>)}
                </ul>
              </div>
            )}
            {packet.action_items.length > 0 && (
              <div>
                <p className="mb-1 flex items-center gap-1.5 text-sm font-medium text-gray-900"><ListTodo className="w-4 h-4" /> Action items → task preview</p>
                <ul className="list-disc pl-6 text-sm text-gray-700">
                  {packet.action_items.map((a, i) => (
                    <li key={i}>
                      {a.title}
                      <span className="text-gray-500">
                        {' '}({a.owner ?? 'no owner'}{a.due_at ? `, due ${a.due_at}` : a.due_words ? `, due “${a.due_words}”` : ''})
                      </span>
                    </li>
                  ))}
                </ul>
              </div>
            )}

            {shares.length > 0 && (
              <div className="text-xs text-gray-500">
                {alreadySent ? 'This exact version was already shared' : 'Shared before'}:{' '}
                {shares.map((s) => (
                  <span key={s.packetId} className="mr-2">
                    {s.channel} on {new Date(s.sharedAt).toLocaleString(undefined, { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })}
                    {s.permalink && (
                      <button type="button" onClick={() => openLink(s.permalink!)} className="ml-1 inline-flex items-center text-blue-600 hover:underline">
                        open <ExternalLink className="ml-0.5 w-3 h-3" />
                      </button>
                    )}
                  </span>
                ))}
              </div>
            )}
          </div>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>Cancel</Button>
          <Button onClick={send} disabled={!packet || !channel.trim() || sending || !!alreadySent}>
            {sending ? <Loader2 className="w-4 h-4 mr-1 animate-spin" /> : <Send className="w-4 h-4 mr-1" />}
            {alreadySent ? 'Already shared' : 'Send'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
