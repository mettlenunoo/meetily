'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { convertFileSrc } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Bookmark as BookmarkIcon, Pause, Play, RotateCcw, RotateCw, X } from 'lucide-react';
import { clock, mediaApi, onSeek, type AudioTrack, type Bookmark } from '@/lib/media';

const SPEEDS = [1, 1.25, 1.5, 2];
const TRACK_LABELS: Record<AudioTrack['kind'], string> = { mixed: 'Everyone', mic: 'Me', system: 'Others' };

interface MeetingPlayerProps {
  meetingId: string;
  tracks: AudioTrack[];
  bookmarks: Bookmark[];
  onBookmarksChange: (bookmarks: Bookmark[]) => void;
  /** Position the player here when it loads (e.g. opened from an action item's time) */
  startAt?: number;
}

/**
 * Plays a meeting's recording. Transcript lines and item times call seekTo() to jump
 * here. The separate tracks let you hear just yourself or just the others. Bookmarks
 * show as marks on the timeline and can be added at the current moment.
 */
export function MeetingPlayer({ meetingId, tracks, bookmarks, onBookmarksChange, startAt }: MeetingPlayerProps) {
  const startedAt = useRef(false);
  const audioRef = useRef<HTMLAudioElement>(null);
  const [track, setTrack] = useState<AudioTrack | null>(tracks.find((t) => t.kind === 'mixed') ?? tracks[0] ?? null);
  const [playing, setPlaying] = useState(false);
  const [time, setTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [speed, setSpeed] = useState(1);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setTrack(tracks.find((t) => t.kind === 'mixed') ?? tracks[0] ?? null);
  }, [tracks]);

  const seek = useCallback((seconds: number, play = true) => {
    const audio = audioRef.current;
    if (!audio) return;
    audio.currentTime = Math.max(0, Math.min(seconds, audio.duration || seconds));
    setTime(audio.currentTime);
    if (play) audio.play().catch(() => undefined);
  }, []);

  useEffect(() => onSeek((s) => seek(s)), [seek]);

  // Switching between Everyone / Me / Others keeps the position.
  const switchTrack = (next: AudioTrack) => {
    const at = audioRef.current?.currentTime ?? 0;
    const wasPlaying = playing;
    setTrack(next);
    requestAnimationFrame(() => {
      const audio = audioRef.current;
      if (!audio) return;
      const resume = () => {
        audio.currentTime = at;
        if (wasPlaying) audio.play().catch(() => undefined);
        audio.removeEventListener('loadedmetadata', resume);
      };
      audio.addEventListener('loadedmetadata', resume);
    });
  };

  const toggle = () => {
    const audio = audioRef.current;
    if (!audio) return;
    if (audio.paused) audio.play().catch((e) => setError(String(e)));
    else audio.pause();
  };

  const addBookmark = async () => {
    try {
      const b = await mediaApi.addBookmark(meetingId, time);
      onBookmarksChange([...bookmarks, b].sort((a, z) => a.seconds - z.seconds));
      toast.success(`Bookmarked ${clock(time)}`);
    } catch (e) {
      toast.error(String(e));
    }
  };

  const removeBookmark = async (id: string) => {
    try {
      await mediaApi.deleteBookmark(id);
      onBookmarksChange(bookmarks.filter((b) => b.id !== id));
    } catch (e) {
      toast.error(String(e));
    }
  };

  if (!track) {
    return (
      <div className="border-b border-gray-200 bg-white px-4 py-2 text-xs text-gray-500">
        No recording file found for this meeting, so it can't be played.
      </div>
    );
  }

  const pct = duration ? (time / duration) * 100 : 0;

  return (
    <div className="border-b border-gray-200 bg-white px-4 py-2">
      <audio
        ref={audioRef}
        src={convertFileSrc(track.path)}
        preload="metadata"
        onPlay={() => setPlaying(true)}
        onPause={() => setPlaying(false)}
        onTimeUpdate={(e) => setTime(e.currentTarget.currentTime)}
        onLoadedMetadata={(e) => {
          setDuration(e.currentTarget.duration);
          e.currentTarget.playbackRate = speed;
          if (startAt != null && !startedAt.current) {
            startedAt.current = true;
            e.currentTarget.currentTime = startAt;
            setTime(startAt);
          }
          setError(null);
        }}
        onError={() => setError('This recording could not be played.')}
      />
      <div className="flex items-center gap-2">
        <button type="button" onClick={() => seek(time - 10, playing)} aria-label="Back 10 seconds" className="p-1 text-gray-500 hover:text-gray-900">
          <RotateCcw className="w-4 h-4" />
        </button>
        <button
          type="button"
          onClick={toggle}
          aria-label={playing ? 'Pause' : 'Play'}
          className="flex h-8 w-8 items-center justify-center rounded-full bg-gray-900 text-white hover:bg-gray-700"
        >
          {playing ? <Pause className="w-4 h-4" /> : <Play className="w-4 h-4 ml-0.5" />}
        </button>
        <button type="button" onClick={() => seek(time + 10, playing)} aria-label="Forward 10 seconds" className="p-1 text-gray-500 hover:text-gray-900">
          <RotateCw className="w-4 h-4" />
        </button>

        <span className="w-12 text-right font-mono text-xs text-gray-500">{clock(time)}</span>
        <div
          className="relative h-2 flex-1 cursor-pointer rounded-full bg-gray-200"
          onClick={(e) => {
            const rect = e.currentTarget.getBoundingClientRect();
            seek(((e.clientX - rect.left) / rect.width) * duration, playing);
          }}
        >
          <div className="absolute inset-y-0 left-0 rounded-full bg-blue-500" style={{ width: `${pct}%` }} />
          {duration > 0 &&
            bookmarks.map((b) => (
              <span
                key={b.id}
                title={`${clock(b.seconds)}${b.note ? ` · ${b.note}` : ''}`}
                className="absolute -top-1 h-4 w-1 rounded bg-amber-500"
                style={{ left: `${(b.seconds / duration) * 100}%` }}
              />
            ))}
        </div>
        <span className="w-12 font-mono text-xs text-gray-500">{clock(duration)}</span>

        <button
          type="button"
          onClick={() => {
            const next = SPEEDS[(SPEEDS.indexOf(speed) + 1) % SPEEDS.length];
            setSpeed(next);
            if (audioRef.current) audioRef.current.playbackRate = next;
          }}
          className="w-11 rounded border border-gray-200 px-1 py-0.5 text-xs text-gray-600 hover:bg-gray-50"
          title="Playback speed"
        >
          {speed}×
        </button>
        {tracks.length > 1 && (
          <div className="flex overflow-hidden rounded border border-gray-200 text-xs" role="group" aria-label="Which voices">
            {tracks.map((t) => (
              <button
                key={t.kind}
                type="button"
                onClick={() => switchTrack(t)}
                className={`px-2 py-0.5 ${t.kind === track.kind ? 'bg-gray-900 text-white' : 'text-gray-600 hover:bg-gray-50'}`}
              >
                {TRACK_LABELS[t.kind]}
              </button>
            ))}
          </div>
        )}
        <button
          type="button"
          onClick={addBookmark}
          className="inline-flex items-center gap-1 rounded border border-amber-200 bg-amber-50 px-2 py-0.5 text-xs text-amber-800 hover:bg-amber-100"
          title="Bookmark this moment"
        >
          <BookmarkIcon className="w-3 h-3" /> Bookmark
        </button>
      </div>
      {error && <p className="mt-1 text-xs text-red-600">{error}</p>}
      {bookmarks.length > 0 && (
        <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
          {bookmarks.map((b) => (
            <span key={b.id} className="group inline-flex items-center gap-1 rounded bg-amber-50 px-1.5 py-0.5 text-xs text-amber-900">
              <button type="button" onClick={() => seek(b.seconds)} className="inline-flex items-center gap-1 hover:underline">
                <BookmarkIcon className="w-3 h-3" />
                {clock(b.seconds)}
                {b.note && <span className="text-amber-700">· {b.note}</span>}
              </button>
              <button
                type="button"
                aria-label="Remove bookmark"
                onClick={() => removeBookmark(b.id)}
                className="hidden text-amber-600 hover:text-red-600 group-hover:inline"
              >
                <X className="w-3 h-3" />
              </button>
            </span>
          ))}
        </div>
      )}
    </div>
  );
}
