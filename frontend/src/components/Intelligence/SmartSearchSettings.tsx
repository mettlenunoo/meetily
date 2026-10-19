'use client';

import { useCallback, useEffect, useState } from 'react';
import Link from 'next/link';
import { listen } from '@tauri-apps/api/event';
import { toast } from 'sonner';
import { Brain, Download, Loader2, RefreshCw } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Switch } from '@/components/ui/switch';
import { intelligenceApi, type IntelligenceStatus } from '@/lib/intelligence';

/** Settings → Smart search: search by meaning with a small local embedding model. */
export function SmartSearchSettings() {
  const [status, setStatus] = useState<IntelligenceStatus | null>(null);
  const [downloading, setDownloading] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(() => intelligenceApi.status().then(setStatus).catch((e) => toast.error(String(e))), []);
  useEffect(() => { load(); }, [load]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<{ modelName: string; progress: number }>('ollama-model-download-progress', (e) => {
      if (status && e.payload.modelName === status.embedModel) setDownloading(Math.round(e.payload.progress));
    }).then((fn) => (unlisten = fn));
    return () => unlisten?.();
  }, [status]);

  if (!status) return <p className="text-sm text-gray-500">Loading…</p>;

  const download = async () => {
    setDownloading(0);
    try {
      await intelligenceApi.downloadModel(status.embedModel);
      toast.success(`${status.embedModel} is ready`);
    } catch (e) {
      toast.error(String(e));
    } finally {
      setDownloading(null);
      load();
    }
  };

  const toggle = async (on: boolean) => {
    setBusy(true);
    try {
      await intelligenceApi.setSemanticSearch(on);
      toast.success(on ? 'Search by meaning is on. Indexing your meetings in the background…' : 'Search by meaning is off');
      load();
    } catch (e) {
      toast.error(String(e));
    } finally {
      setBusy(false);
    }
  };

  const reindex = async () => {
    setBusy(true);
    try {
      const [meetings, passages] = await intelligenceApi.reindex();
      toast.success(meetings ? `Indexed ${meetings} meeting(s), ${passages} passage(s)` : 'Everything is already indexed');
      load();
    } catch (e) {
      toast.error(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="max-w-2xl space-y-5">
      <div>
        <h3 className="flex items-center gap-2 text-lg font-semibold text-gray-900">
          <Brain className="w-5 h-5" /> Search by meaning
        </h3>
        <p className="mt-1 text-sm text-gray-600">
          Finds passages by what they mean, not just the words used. “Money plan for next year” finds “the 2027 budget
          forecast”. It powers Ask Hearken and the “related by meaning” results in search. A small model
          ({status.embedModel}, about 270 MB) runs locally in Ollama; nothing leaves this Mac.
        </p>
      </div>

      {!status.modelInstalled ? (
        <Button onClick={download} disabled={downloading !== null}>
          {downloading !== null ? <Loader2 className="w-4 h-4 mr-1 animate-spin" /> : <Download className="w-4 h-4 mr-1" />}
          {downloading !== null ? `Downloading… ${downloading}%` : `Download ${status.embedModel}`}
        </Button>
      ) : (
        <div className="flex items-center justify-between rounded-md border border-gray-200 p-3">
          <div>
            <p className="text-sm font-medium text-gray-900">Search by meaning</p>
            <p className="text-xs text-gray-500">
              {status.indexedMeetings} of {status.totalMeetings} meetings indexed ({status.passages} passages). New and
              changed meetings are indexed after each summary and every 20 minutes.
            </p>
          </div>
          <Switch checked={status.semanticSearch} disabled={busy} onCheckedChange={toggle} />
        </div>
      )}

      {status.semanticSearch && (
        <Button variant="outline" onClick={reindex} disabled={busy}>
          {busy ? <Loader2 className="w-4 h-4 mr-1 animate-spin" /> : <RefreshCw className="w-4 h-4 mr-1" />} Index changed meetings now
        </Button>
      )}
      <Link href="/model-check" className="block text-sm text-blue-700 hover:underline">
        Compare transcription models on your own recordings (accuracy and speed) →
      </Link>
      <p className="text-xs text-gray-500">
        Ask Hearken works without this too (it then uses keyword search), but finds less. Answers use your summary
        model; with a cloud model, local-only projects are left out.
      </p>
    </div>
  );
}
