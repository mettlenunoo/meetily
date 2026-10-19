'use client';

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { AlertTriangle, CheckCircle2, HardDriveDownload, ShieldCheck } from 'lucide-react';
import { Button } from '@/components/ui/button';

interface BackupStatus {
  timeMachineConfigured: boolean;
  timeMachineLatest: string | null;
  lastManualBackup: string | null;
  databasePath: string | null;
  recordingsFolder: string | null;
  protected: boolean;
}

/** Settings → Backups: is this data protected, and a one-click copy of the database. */
export function BackupSettingsPanel() {
  const [status, setStatus] = useState<BackupStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const load = () => invoke<BackupStatus>('api_backup_status').then(setStatus).catch((e) => toast.error(String(e)));
  useEffect(() => { load(); }, []);
  if (!status) return <p className="text-sm text-gray-500">Loading…</p>;

  const backup = async () => {
    setBusy(true);
    try {
      const path = await invoke<string | null>('api_backup_database');
      if (path) toast.success('Backup saved', { description: path });
      load();
    } catch (e) {
      toast.error(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="max-w-2xl space-y-5">
      <h3 className="flex items-center gap-2 text-lg font-semibold text-gray-900"><ShieldCheck className="w-5 h-5" /> Backups</h3>
      {status.protected ? (
        <p className="flex items-start gap-2 rounded-md bg-green-50 p-3 text-sm text-green-900">
          <CheckCircle2 className="mt-0.5 w-4 h-4" />
          {status.timeMachineConfigured
            ? `Time Machine is backing up this Mac${status.timeMachineLatest ? ` (last backup ${status.timeMachineLatest})` : ''}. That includes Hearken's database and your recordings.`
            : `You saved a copy of the database recently. Remember the recordings folder needs backing up too.`}
        </p>
      ) : (
        <p className="flex items-start gap-2 rounded-md bg-amber-50 p-3 text-sm text-amber-900">
          <AlertTriangle className="mt-0.5 w-4 h-4" />
          Nothing is backing up your recordings and meeting records. If this Mac is lost or its disk fails, they’re gone.
          The simplest fix is Time Machine with an external drive (System Settings → General → Time Machine).
        </p>
      )}
      <div className="space-y-2 text-sm text-gray-700">
        <p><span className="text-gray-500">Database:</span> <code className="text-xs">{status.databasePath}</code></p>
        <p><span className="text-gray-500">Recordings:</span> <code className="text-xs">{status.recordingsFolder}</code></p>
        {status.lastManualBackup && (
          <p className="text-xs text-gray-500">Last copy saved from here: {new Date(status.lastManualBackup).toLocaleString()}</p>
        )}
      </div>
      <Button onClick={backup} disabled={busy}>
        <HardDriveDownload className="w-4 h-4 mr-1" /> Save a copy of the database…
      </Button>
      <p className="text-xs text-gray-500">
        A complete, consistent copy of meetings, transcripts, summaries, items, projects and documents, safe to take while
        Hearken is running. Audio files stay in the recordings folder; copy that folder too, or use Time Machine.
      </p>
    </div>
  );
}
