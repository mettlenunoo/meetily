'use client';

import { useEffect, useState } from 'react';
import { toast } from 'sonner';
import { CheckCircle2, Loader2, MessageSquare, RefreshCw } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { slackApi, type BotLink } from '@/lib/slack';

/** Settings → Slack bot: how Hearken reaches Desmond's bot (this Mac or a server over SSH). */
export function BotLinkSettings() {
  const [link, setLink] = useState<BotLink | null>(null);
  const [configured, setConfigured] = useState(false);
  const [busy, setBusy] = useState<'test' | 'save' | 'sync' | null>(null);
  const [status, setStatus] = useState<string | null>(null);

  useEffect(() => {
    slackApi.getLink().then((s) => { setLink(s.link); setConfigured(s.configured); }).catch((e) => toast.error(String(e)));
  }, []);

  if (!link) return <p className="text-sm text-gray-500">Loading…</p>;
  const set = (changes: Partial<BotLink>) => setLink({ ...link, ...changes });

  const test = async () => {
    setBusy('test');
    setStatus(null);
    try {
      const r = await slackApi.test(link);
      setStatus(r.slack ? `Connected. The bot knows ${r.projects} project(s).` : 'Connected, but the bot has no SLACK_BOT_TOKEN in its .env yet.');
    } catch (e) {
      setStatus(`Not connected: ${e}`);
    } finally {
      setBusy(null);
    }
  };

  const save = async () => {
    setBusy('save');
    try {
      await slackApi.saveLink(link);
      setConfigured(true);
      toast.success('Saved');
    } catch (e) {
      toast.error(String(e));
    } finally {
      setBusy(null);
    }
  };

  const sync = async () => {
    setBusy('sync');
    try {
      const r = await slackApi.syncProjects();
      if (r.unresolved_channels.length) {
        toast.warning(`Sent ${r.projects} project(s). The bot can't see ${r.unresolved_channels.join(', ')}`, {
          description: 'Check the channel names, and invite the bot there with /invite @Desmonds Bot.',
        });
      } else {
        toast.success(`Sent ${r.projects} project(s) to the bot`);
      }
    } catch (e) {
      toast.error(String(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="max-w-2xl space-y-6">
      <div>
        <h3 className="flex items-center gap-2 text-lg font-semibold text-gray-900">
          <MessageSquare className="w-5 h-5" /> Slack bot
        </h3>
        <p className="mt-1 text-sm text-gray-600">
          Connect to Desmond’s bot to share a meeting into its project’s Slack channel and to see the bot’s tasks and
          decisions on project pages. Hearken runs the bot’s <code className="text-xs">hearken_bridge.py</code> directly,
          so there’s no server to open. Nothing is sent until you press “Send to Slack” on a meeting, and transcripts,
          audio and local-only projects are never sent.
        </p>
      </div>

      <div className="space-y-2">
        <Label>Where the bot runs</Label>
        <div className="flex gap-2">
          {(['local', 'ssh'] as const).map((m) => (
            <button
              key={m}
              type="button"
              onClick={() => set({ mode: m })}
              className={`rounded-md border px-3 py-1.5 text-sm ${link.mode === m ? 'border-blue-500 bg-blue-50 text-blue-800' : 'border-gray-200 text-gray-700 hover:bg-gray-50'}`}
            >
              {m === 'local' ? 'On this Mac' : 'On a server (SSH)'}
            </button>
          ))}
        </div>
      </div>

      {link.mode === 'ssh' && (
        <div className="space-y-1.5">
          <Label htmlFor="bot-host">SSH host</Label>
          <Input id="bot-host" value={link.sshHost ?? ''} onChange={(e) => set({ sshHost: e.target.value })} placeholder="my-vps (from ~/.ssh/config) or user@203.0.113.5" />
          <p className="text-xs text-gray-500">Uses your SSH keys; password logins aren’t supported.</p>
        </div>
      )}
      <div className="space-y-1.5">
        <Label htmlFor="bot-python">The bot’s Python</Label>
        <Input id="bot-python" value={link.python} onChange={(e) => set({ python: e.target.value })} placeholder={link.mode === 'ssh' ? '/opt/desmonds-bot/.venv/bin/python' : ''} />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="bot-script">hearken_bridge.py</Label>
        <Input id="bot-script" value={link.script} onChange={(e) => set({ script: e.target.value })} placeholder={link.mode === 'ssh' ? '/opt/desmonds-bot/hearken_bridge.py' : ''} />
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <Button variant="outline" onClick={test} disabled={busy !== null}>
          {busy === 'test' ? <Loader2 className="w-4 h-4 mr-1 animate-spin" /> : <CheckCircle2 className="w-4 h-4 mr-1" />} Test connection
        </Button>
        <Button onClick={save} disabled={busy !== null}>Save</Button>
        {configured && (
          <Button variant="outline" onClick={sync} disabled={busy !== null}>
            {busy === 'sync' ? <Loader2 className="w-4 h-4 mr-1 animate-spin" /> : <RefreshCw className="w-4 h-4 mr-1" />} Send projects to the bot
          </Button>
        )}
      </div>
      {status && <p className="text-sm text-gray-700">{status}</p>}
    </div>
  );
}
