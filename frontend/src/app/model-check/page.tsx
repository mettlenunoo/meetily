'use client';

import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { toast } from 'sonner';
import { Award, FlaskConical, Loader2, Play } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Textarea } from '@/components/ui/textarea';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import type { Transcript } from '@/types';

interface ModelChoice { engine: 'whisper' | 'parakeet'; model: string }
interface EvalResult {
  id: string; engine: string; model: string; referenceWords: number; errors: number; wer: number;
  hypothesis: string; audioSeconds: number; secondsTaken: number; createdAt: string;
}
interface ModelScore { engine: string; model: string; clips: number; referenceWords: number; wer: number; speed: number }
interface Comparison { scores: ModelScore[]; recommended: ModelScore | null; recent: EvalResult[] }
interface ModelInfo { name: string; status: unknown }

const pct = (x: number) => `${(x * 100).toFixed(1)}%`;
const parseClock = (s: string) => {
  const parts = s.trim().split(':').map(Number);
  if (parts.some(isNaN)) return NaN;
  return parts.reduce((acc, p) => acc * 60 + p, 0);
};

/**
 * Which transcription model is most accurate on your recordings (e.g. Ghanaian-accented
 * English)? Correct a short stretch of a transcript; each model transcribes the same audio
 * and is scored against your text.
 */
export default function ModelCheckPage() {
  const { meetings } = useSidebar();
  const realMeetings = meetings.filter((m) => m.id.includes('-') && !m.id.startsWith('intro-call'));
  const [meetingId, setMeetingId] = useState('');
  const [from, setFrom] = useState('0:00');
  const [length, setLength] = useState('2:00');
  const [reference, setReference] = useState('');
  const [available, setAvailable] = useState<ModelChoice[]>([]);
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const [running, setRunning] = useState<string | null>(null);
  const [results, setResults] = useState<EvalResult[]>([]);
  const [summary, setSummary] = useState<Comparison | null>(null);

  const key = (m: ModelChoice) => `${m.engine}:${m.model}`;

  useEffect(() => {
    const isAvailable = (m: ModelInfo) => m.status === 'Available';
    Promise.all([
      invoke<ModelInfo[]>('whisper_get_available_models').catch(() => []),
      invoke<ModelInfo[]>('parakeet_get_available_models').catch(() => []),
    ]).then(([w, p]) => {
      const list: ModelChoice[] = [
        ...p.filter(isAvailable).map((m) => ({ engine: 'parakeet' as const, model: m.name })),
        ...w.filter(isAvailable).map((m) => ({ engine: 'whisper' as const, model: m.name })),
      ];
      setAvailable(list);
      setChosen(new Set(list.map(key)));
    });
    invoke<Comparison>('api_model_comparison').then(setSummary).catch(console.error);
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<{ index: number; total: number; model: string }>('model-compare-progress', (e) =>
      setRunning(`${e.payload.model} (${e.payload.index + 1} of ${e.payload.total})`),
    ).then((fn) => (unlisten = fn));
    return () => unlisten?.();
  }, []);

  const range = useMemo(() => {
    const start = parseClock(from);
    const len = parseClock(length);
    return { start, end: start + len, valid: !isNaN(start) && !isNaN(len) && len > 0 && len <= 600 };
  }, [from, length]);

  // Pre-fill the box with Hearken's own transcript of that stretch, for you to correct.
  const loadTranscript = async () => {
    if (!meetingId || !range.valid) return;
    try {
      const data = await invoke<{ transcripts: Transcript[] }>('api_get_meeting_transcripts', { meetingId, limit: 100000, offset: 0 });
      const text = data.transcripts
        .filter((t) => (t.audio_start_time ?? 0) >= range.start && (t.audio_start_time ?? 0) < range.end)
        .sort((a, b) => (a.audio_start_time ?? 0) - (b.audio_start_time ?? 0))
        .map((t) => t.text.trim())
        .join(' ');
      setReference(text);
      if (!text) toast.info('No transcript in that range; type what was said.');
    } catch (e) {
      toast.error(String(e));
    }
  };

  const run = async () => {
    const models = available.filter((m) => chosen.has(key(m)));
    setRunning('starting…');
    try {
      const r = await invoke<EvalResult[]>('api_compare_models', {
        meetingId, fromSeconds: range.start, toSeconds: range.end, reference, models,
      });
      setResults(r);
      setSummary(await invoke<Comparison>('api_model_comparison'));
    } catch (e) {
      toast.error(String(e));
    } finally {
      setRunning(null);
    }
  };

  const wordCount = reference.trim() ? reference.trim().split(/\s+/).length : 0;

  return (
    <div className="h-screen overflow-y-auto bg-gray-50">
      <div className="mx-auto max-w-4xl px-6 py-8 space-y-6">
        <header>
          <h1 className="flex items-center gap-2 text-2xl font-semibold text-gray-900">
            <FlaskConical className="w-6 h-6" /> Which transcription model is best for you?
          </h1>
          <p className="mt-1 text-sm text-gray-500">
            Pick a stretch of one of your recordings, correct its transcript to exactly what was said, and each downloaded
            model transcribes the same audio. Lower error rate is better. Try several clips with different speakers and
            accents; the recommendation adds them all up.
          </p>
        </header>

        {summary && summary.scores.length > 0 && (
          <section className="rounded-lg border border-gray-200 bg-white p-4">
            <h2 className="mb-3 text-sm font-semibold text-gray-900">Across all your clips</h2>
            {summary.recommended && (
              <p className="mb-3 flex items-center gap-2 rounded-md bg-green-50 px-3 py-2 text-sm text-green-900">
                <Award className="w-4 h-4" /> Recommended: <strong>{summary.recommended.model}</strong> ({summary.recommended.engine}),
                {' '}{pct(summary.recommended.wer)} word errors, {summary.recommended.speed.toFixed(0)}× real time.
                Set it in Settings → Transcription.
              </p>
            )}
            <table className="w-full text-sm">
              <thead><tr className="text-left text-xs text-gray-500"><th className="py-1">Model</th><th>Clips</th><th>Words checked</th><th>Word errors</th><th>Speed</th></tr></thead>
              <tbody>
                {summary.scores.map((s) => (
                  <tr key={`${s.engine}:${s.model}`} className="border-t border-gray-100">
                    <td className="py-1.5">{s.model} <span className="text-xs text-gray-400">{s.engine}</span></td>
                    <td>{s.clips}</td><td>{s.referenceWords}</td><td>{pct(s.wer)}</td><td>{s.speed.toFixed(1)}×</td>
                  </tr>
                ))}
              </tbody>
            </table>
            {!summary.recommended && <p className="mt-2 text-xs text-gray-500">A recommendation appears after at least 100 words checked for a model that runs at least 2× real time.</p>}
          </section>
        )}

        <section className="space-y-4 rounded-lg border border-gray-200 bg-white p-4">
          <div className="grid grid-cols-1 gap-3 md:grid-cols-[1fr_110px_110px]">
            <div className="space-y-1.5">
              <Label>Recording</Label>
              <Select value={meetingId} onValueChange={setMeetingId}>
                <SelectTrigger><SelectValue placeholder="Choose a recording" /></SelectTrigger>
                <SelectContent>{realMeetings.map((m) => <SelectItem key={m.id} value={m.id}>{m.title}</SelectItem>)}</SelectContent>
              </Select>
            </div>
            <div className="space-y-1.5"><Label htmlFor="from">Start (m:ss)</Label><Input id="from" value={from} onChange={(e) => setFrom(e.target.value)} /></div>
            <div className="space-y-1.5"><Label htmlFor="len">Length</Label><Input id="len" value={length} onChange={(e) => setLength(e.target.value)} /></div>
          </div>
          <div className="space-y-1.5">
            <div className="flex items-center justify-between">
              <Label htmlFor="ref">Exactly what was said ({wordCount} words)</Label>
              <Button size="sm" variant="ghost" className="h-7" onClick={loadTranscript} disabled={!meetingId || !range.valid}>Fill in Hearken’s transcript to correct</Button>
            </div>
            <Textarea id="ref" rows={6} value={reference} onChange={(e) => setReference(e.target.value)} placeholder="Listen to the clip in the meeting’s player and type or correct the words. Names and Twi/Ga words count too." />
          </div>
          <div className="space-y-1.5">
            <Label>Models to compare</Label>
            {available.length === 0 ? (
              <p className="text-sm text-gray-500">No downloaded models found. Download Whisper or Parakeet models in Settings → Transcription.</p>
            ) : (
              <div className="flex flex-wrap gap-2">
                {available.map((m) => (
                  <label key={key(m)} className="flex items-center gap-1.5 rounded-md border border-gray-200 px-2 py-1 text-sm">
                    <input type="checkbox" checked={chosen.has(key(m))} onChange={(e) => setChosen((s) => { const n = new Set(s); e.target.checked ? n.add(key(m)) : n.delete(key(m)); return n; })} />
                    {m.model} <span className="text-xs text-gray-400">{m.engine}</span>
                  </label>
                ))}
              </div>
            )}
          </div>
          <Button onClick={run} disabled={!meetingId || !range.valid || wordCount < 20 || chosen.size === 0 || running !== null}>
            {running ? <Loader2 className="w-4 h-4 mr-1 animate-spin" /> : <Play className="w-4 h-4 mr-1" />}
            {running ? `Transcribing with ${running}` : 'Compare'}
          </Button>
          <p className="text-xs text-gray-500">Runs on this Mac. Not possible while recording. Your chosen transcription model is put back afterwards.</p>
        </section>

        {results.length > 0 && (
          <section className="rounded-lg border border-gray-200 bg-white p-4">
            <h2 className="mb-3 text-sm font-semibold text-gray-900">This clip</h2>
            <ul className="space-y-3">
              {[...results].sort((a, b) => a.wer - b.wer).map((r) => (
                <li key={r.id} className="text-sm">
                  <p className="font-medium text-gray-900">
                    {r.model} <span className="text-xs text-gray-400">{r.engine}</span>
                    <span className="ml-3 text-gray-700">{pct(r.wer)} errors ({r.errors}/{r.referenceWords} words)</span>
                    <span className="ml-3 text-gray-500">{(r.audioSeconds / Math.max(r.secondsTaken, 0.001)).toFixed(1)}× real time</span>
                  </p>
                  <p className="mt-1 text-xs italic text-gray-600">{r.hypothesis}</p>
                </li>
              ))}
            </ul>
          </section>
        )}
      </div>
    </div>
  );
}
