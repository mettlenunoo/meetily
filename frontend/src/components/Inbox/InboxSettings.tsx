'use client';

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { FolderOpen, Inbox } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';

interface InboxSettings { enabled: boolean; folder: string }
interface InboxStatus { settings: InboxSettings; folderExists: boolean; waiting: number; imported: number; failed: number }

/** Settings → Inbox: auto-import recordings made on a phone, iPad or recorder app. */
export function InboxSettingsPanel() {
  const [status, setStatus] = useState<InboxStatus | null>(null);
  const [folder, setFolder] = useState('');

  useEffect(() => {
    invoke<InboxStatus>('api_inbox_status').then((s) => { setStatus(s); setFolder(s.settings.folder); }).catch((e) => toast.error(String(e)));
  }, []);
  if (!status) return <p className="text-sm text-gray-500">Loading…</p>;

  const save = async (enabled: boolean) => {
    try {
      setStatus(await invoke<InboxStatus>('api_set_inbox', { settings: { enabled, folder } }));
      if (enabled) toast.success('Inbox is on', { description: 'New recordings in the folder are imported within a minute or two.' });
    } catch (e) {
      toast.error(String(e));
    }
  };

  return (
    <div className="max-w-2xl space-y-5">
      <div>
        <h3 className="flex items-center gap-2 text-lg font-semibold text-gray-900"><Inbox className="w-5 h-5" /> Hearken Inbox</h3>
        <p className="mt-1 text-sm text-gray-600">
          Record on your iPhone, iPad or any recorder app, save the file into this folder, and Hearken imports and transcribes
          it automatically, as if you had recorded on the Mac. The folder is in iCloud Drive, so it syncs from your phone by
          itself. Nothing else goes online. Imported files are moved to “Imported”, never deleted.
        </p>
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="inbox-folder">Folder</Label>
        <div className="flex gap-2">
          <Input id="inbox-folder" value={folder} onChange={(e) => setFolder(e.target.value)} />
          <Button variant="outline" onClick={() => invoke('api_reveal_inbox').catch((e) => toast.error(String(e)))}>
            <FolderOpen className="w-4 h-4 mr-1" /> Open
          </Button>
        </div>
      </div>
      <div className="flex items-center justify-between rounded-md border border-gray-200 p-3">
        <div>
          <p className="text-sm font-medium text-gray-900">Import new recordings automatically</p>
          <p className="text-xs text-gray-500">
            {status.settings.enabled
              ? `${status.waiting} waiting · ${status.imported} imported${status.failed ? ` · ${status.failed} couldn't be imported` : ''}`
              : 'Off'}
            {' '}· not while you’re recording
          </p>
        </div>
        <Switch checked={status.settings.enabled} onCheckedChange={save} />
      </div>
      <div className="rounded-md bg-gray-50 p-3 text-sm text-gray-700">
        <p className="font-medium text-gray-900">On your iPhone</p>
        <ol className="mt-1 list-decimal space-y-1 pl-5">
          <li>Record in Voice Memos (or any recorder app).</li>
          <li>Tap the recording → Share → Save to Files → iCloud Drive → <strong>Hearken Inbox</strong> → Save.</li>
          <li>Name the file after the meeting (e.g. “Budget review”): that becomes its title in Hearken.</li>
        </ol>
        <p className="mt-2 text-xs text-gray-500">
          Tip: in the Shortcuts app, make a share-sheet shortcut with one action, “Save File” to the Hearken Inbox folder,
          so it’s one tap from any app.
        </p>
      </div>
    </div>
  );
}
