'use client';

import { useState } from 'react';
import { useRouter } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { CheckCircle2, Circle, FolderKanban, Quote, ShieldAlert, User, Video } from 'lucide-react';
import { DueDateButton } from './DueDateButton';
import type { TrackedItem } from '@/lib/projects';

function formatClock(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  const h = Math.floor(total / 3600);
  const mm = String(Math.floor((total % 3600) / 60)).padStart(2, '0');
  const ss = String(total % 60).padStart(2, '0');
  return h > 0 ? `${String(h).padStart(2, '0')}:${mm}:${ss}` : `${mm}:${ss}`;
}

function shortDate(iso: string): string {
  const d = new Date(iso);
  return isNaN(d.getTime()) ? iso.slice(0, 10) : d.toLocaleDateString(undefined, { day: 'numeric', month: 'short' });
}

interface ActionItemRowProps {
  item: TrackedItem;
  /** Called with the changed fields after a successful save */
  onChanged: (id: string, changes: Partial<TrackedItem>) => void;
  showProject?: boolean;
}

/** One action item with its meeting, project, owner, due date and supporting quote. */
export function ActionItemRow({ item, onChanged, showProject = true }: ActionItemRowProps) {
  const router = useRouter();
  const [editingOwner, setEditingOwner] = useState<string | null>(null);

  const save = async (update: Record<string, string>, optimistic: Partial<TrackedItem>) => {
    onChanged(item.id, optimistic);
    try {
      await invoke('api_update_meeting_item', { id: item.id, update });
    } catch (error) {
      toast.error('Could not update item');
      console.error(error);
      onChanged(item.id, item); // roll back
    }
  };

  const commitOwner = () => {
    if (editingOwner === null) return;
    const owner = editingOwner.trim();
    setEditingOwner(null);
    if (owner !== (item.owner ?? '')) save({ owner }, { owner: owner || null });
  };

  const done = item.status === 'done';
  return (
    <li className="group flex items-start gap-2.5 py-2.5">
      <button
        type="button"
        aria-label={done ? 'Mark as open' : 'Mark as done'}
        onClick={() => save({ status: done ? 'open' : 'done' }, { status: done ? 'open' : 'done' })}
        className="mt-0.5 text-gray-400 hover:text-green-600"
      >
        {done ? <CheckCircle2 className="w-4 h-4 text-green-600" /> : <Circle className="w-4 h-4" />}
      </button>
      <div className="flex-1 min-w-0">
        <p className={`text-sm ${done ? 'line-through text-gray-400' : 'text-gray-900'}`}>{item.title}</p>
        <div className="mt-1 flex flex-wrap items-center gap-1.5 text-xs">
          {editingOwner !== null ? (
            <input
              autoFocus
              value={editingOwner}
              placeholder="Owner"
              onChange={(e) => setEditingOwner(e.target.value)}
              onBlur={commitOwner}
              onKeyDown={(e) => {
                if (e.key === 'Enter') commitOwner();
                if (e.key === 'Escape') setEditingOwner(null);
              }}
              className="h-6 w-32 rounded border border-gray-300 px-1.5 text-xs"
            />
          ) : (
            <button
              type="button"
              onClick={() => setEditingOwner(item.owner ?? '')}
              className={`inline-flex items-center gap-1 rounded px-1.5 py-0.5 border ${
                item.owner ? 'border-blue-200 bg-blue-50 text-blue-700' : 'border-dashed border-gray-300 text-gray-400'
              }`}
            >
              <User className="w-3 h-3" />
              {item.owner ?? 'No owner'}
            </button>
          )}
          <DueDateButton
            item={item}
            onChange={(dueAt) => save({ dueAt }, { dueAt: dueAt || null })}
          />
          {showProject && item.projectName && (
            <button
              type="button"
              onClick={() => router.push(`/project?id=${item.projectId}`)}
              className="inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-gray-600 hover:bg-gray-100"
            >
              <FolderKanban className="w-3 h-3" /> {item.projectName}
            </button>
          )}
          <button
            type="button"
            onClick={() => router.push(`/meeting-details?id=${item.meetingId}`)}
            className="inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-gray-500 hover:bg-gray-100 hover:text-gray-800"
            title="Open the meeting"
          >
            <Video className="w-3 h-3" /> {item.meetingTitle} · {shortDate(item.meetingCreatedAt)}
          </button>
        </div>
        {item.evidenceQuote && (
          <div className="mt-1.5 flex items-start gap-1.5 text-xs text-gray-500">
            <Quote className="w-3 h-3 mt-0.5 flex-shrink-0" />
            <span>
              {item.evidenceStartTime != null && (
                <span className="font-mono text-gray-400 mr-1">[{formatClock(item.evidenceStartTime)}]</span>
              )}
              <span className="italic">“{item.evidenceQuote}”</span>
              {!item.evidenceVerified && (
                <span className="ml-1.5 inline-flex items-center gap-0.5 text-amber-600" title="Quote not found in the transcript">
                  <ShieldAlert className="w-3 h-3" /> not found in transcript
                </span>
              )}
            </span>
          </div>
        )}
      </div>
    </li>
  );
}
