'use client';

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { CheckCircle2, Circle, Gavel, ListTodo, Plus, Quote, ShieldAlert, Trash2, User } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { DueDateButton } from '@/components/ActionItems/DueDateButton';
import { seekTo } from '@/lib/media';

export interface MeetingItem {
  id: string;
  meetingId: string;
  kind: 'action' | 'decision';
  title: string;
  description: string | null;
  owner: string | null;
  /** As spoken, e.g. "Friday" */
  dueDate: string | null;
  /** The exact date, YYYY-MM-DD, when known */
  dueAt: string | null;
  priority: 'high' | 'medium' | 'low' | null;
  status: 'open' | 'done' | 'dismissed';
  evidenceQuote: string | null;
  evidenceStartTime: number | null;
  evidenceVerified: boolean;
  source: 'ai' | 'user';
  userEdited: boolean;
  createdAt: string;
  updatedAt: string;
}

function formatClock(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const mm = String(m).padStart(2, '0');
  const ss = String(s).padStart(2, '0');
  return h > 0 ? `${String(h).padStart(2, '0')}:${mm}:${ss}` : `${mm}:${ss}`;
}

interface MeetingItemsPanelProps {
  meetingId: string;
  /** Changes when a summary run finishes, so the list reloads with fresh items */
  refreshKey?: string;
}

/**
 * Action items and decisions for a meeting. Each AI item shows the transcript
 * quote it came from and when it was said; items whose quote could not be found
 * in the transcript are flagged for review.
 */
export function MeetingItemsPanel({ meetingId, refreshKey }: MeetingItemsPanelProps) {
  const [items, setItems] = useState<MeetingItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [newAction, setNewAction] = useState('');
  // Inline editor for owner / due date (window.prompt is unavailable in the Tauri webview)
  const [editing, setEditing] = useState<{ id: string; field: 'owner'; value: string } | null>(null);

  const load = useCallback(async () => {
    try {
      const result = await invoke<MeetingItem[]>('api_get_meeting_items', { meetingId });
      setItems(result);
    } catch (error) {
      console.error('Failed to load action items:', error);
    } finally {
      setLoading(false);
    }
  }, [meetingId]);

  useEffect(() => {
    setLoading(true);
    load();
  }, [load, refreshKey]);

  const update = async (id: string, update: Partial<Pick<MeetingItem, 'title' | 'owner' | 'dueDate' | 'dueAt' | 'status'>>) => {
    // Optimistic update; reload on failure
    setItems((prev) => prev.map((it) => (it.id === id ? { ...it, ...update } : it)));
    try {
      // The backend treats '' as "clear this field"
      const payload = 'dueAt' in update ? { ...update, dueAt: update.dueAt ?? '' } : update;
      await invoke('api_update_meeting_item', { id, update: payload });
    } catch (error) {
      toast.error('Could not update item');
      console.error(error);
      load();
    }
  };

  const remove = async (id: string) => {
    setItems((prev) => prev.filter((it) => it.id !== id));
    try {
      await invoke('api_delete_meeting_item', { id });
    } catch (error) {
      toast.error('Could not delete item');
      console.error(error);
      load();
    }
  };

  const addAction = async () => {
    const title = newAction.trim();
    if (!title) return;
    try {
      const created = await invoke<MeetingItem>('api_create_meeting_item', {
        meetingId,
        kind: 'action',
        title,
        owner: null,
        dueDate: null,
      });
      setItems((prev) => [...prev, created]);
      setNewAction('');
    } catch (error) {
      toast.error('Could not add action item');
      console.error(error);
    }
  };

  const startEdit = (item: MeetingItem, field: 'owner') =>
    setEditing({ id: item.id, field, value: item[field] ?? '' });

  const commitEdit = () => {
    if (!editing) return;
    const { id, field } = editing;
    const value = editing.value.trim();
    setEditing(null);
    setItems((prev) => prev.map((it) => (it.id === id ? { ...it, [field]: value || null } : it)));
    // An empty string tells the backend to clear the field
    invoke('api_update_meeting_item', { id, update: { [field]: value } }).catch((error) => {
      toast.error('Could not update item');
      console.error(error);
      load();
    });
  };

  const actions = items.filter((i) => i.kind === 'action' && i.status !== 'dismissed');
  const decisions = items.filter((i) => i.kind === 'decision' && i.status !== 'dismissed');
  const openCount = actions.filter((a) => a.status === 'open').length;

  if (loading && items.length === 0) {
    return null;
  }

  const renderEvidence = (item: MeetingItem) => {
    if (!item.evidenceQuote) return null;
    return (
      <div className="mt-1.5 flex items-start gap-1.5 text-xs text-gray-500">
        <Quote className="w-3 h-3 mt-0.5 flex-shrink-0" />
        <span>
          {item.evidenceStartTime != null && (
            <button
              type="button"
              onClick={() => seekTo(item.evidenceStartTime!)}
              title="Play from here"
              className="font-mono text-gray-400 mr-1 hover:text-blue-600 hover:underline"
            >
              [{formatClock(item.evidenceStartTime)}]
            </button>
          )}
          <span className="italic">“{item.evidenceQuote}”</span>
          {!item.evidenceVerified && (
            <span
              className="ml-1.5 inline-flex items-center gap-0.5 text-amber-600"
              title="This quote could not be found in the transcript. Check it before relying on this item."
            >
              <ShieldAlert className="w-3 h-3" /> not found in transcript
            </span>
          )}
        </span>
      </div>
    );
  };

  const renderEditor = (item: MeetingItem, field: 'owner', placeholder: string) =>
    editing && editing.id === item.id && editing.field === field ? (
      <input
        autoFocus
        value={editing.value}
        placeholder={placeholder}
        onChange={(e) => setEditing({ ...editing, value: e.target.value })}
        onBlur={commitEdit}
        onKeyDown={(e) => {
          if (e.key === 'Enter') commitEdit();
          if (e.key === 'Escape') setEditing(null);
        }}
        className="h-6 w-32 rounded border border-gray-300 px-1.5 text-xs"
      />
    ) : null;

  const renderMeta = (item: MeetingItem) => (
    <div className="mt-1 flex flex-wrap items-center gap-1.5 text-xs">
      {renderEditor(item, 'owner', 'Owner') ?? (
      <button
        type="button"
        onClick={() => startEdit(item, 'owner')}
        className={`inline-flex items-center gap-1 rounded px-1.5 py-0.5 border ${
          item.owner ? 'border-blue-200 bg-blue-50 text-blue-700' : 'border-dashed border-gray-300 text-gray-400'
        }`}
      >
        <User className="w-3 h-3" />
        {item.owner ?? 'No owner'}
      </button>
      )}
      {item.kind === 'action' && (
        <DueDateButton item={item} onChange={(dueAt) => update(item.id, { dueAt: dueAt || null })} />
      )}
      {item.priority && (
        <span
          className={`rounded px-1.5 py-0.5 ${
            item.priority === 'high'
              ? 'bg-red-50 text-red-700'
              : item.priority === 'medium'
                ? 'bg-amber-50 text-amber-700'
                : 'bg-gray-100 text-gray-600'
          }`}
        >
          {item.priority}
        </span>
      )}
      {item.source === 'user' && <span className="text-gray-400">added by you</span>}
    </div>
  );

  return (
    <section className="mt-8 border-t border-gray-200 pt-6 space-y-6" aria-label="Action items and decisions">
      <div>
        <h3 className="flex items-center gap-2 text-sm font-semibold text-gray-900 mb-3">
          <ListTodo className="w-4 h-4" />
          Action items
          <span className="text-xs font-normal text-gray-500">
            {openCount} open{actions.length > openCount ? `, ${actions.length - openCount} done` : ''}
          </span>
        </h3>

        {actions.length === 0 && (
          <p className="text-sm text-gray-500 mb-3">
            No action items yet. They are extracted automatically when the summary is generated.
          </p>
        )}

        <ul className="space-y-3">
          {actions.map((item) => (
            <li key={item.id} className="group flex items-start gap-2">
              <button
                type="button"
                aria-label={item.status === 'done' ? 'Mark as open' : 'Mark as done'}
                onClick={() => update(item.id, { status: item.status === 'done' ? 'open' : 'done' })}
                className="mt-0.5 text-gray-400 hover:text-green-600"
              >
                {item.status === 'done' ? (
                  <CheckCircle2 className="w-4 h-4 text-green-600" />
                ) : (
                  <Circle className="w-4 h-4" />
                )}
              </button>
              <div className="flex-1 min-w-0">
                <p className={`text-sm ${item.status === 'done' ? 'line-through text-gray-400' : 'text-gray-900'}`}>
                  {item.title}
                </p>
                {item.description && <p className="text-xs text-gray-600 mt-0.5">{item.description}</p>}
                {renderMeta(item)}
                {renderEvidence(item)}
              </div>
              <button
                type="button"
                aria-label="Remove item"
                onClick={() => (item.source === 'user' ? remove(item.id) : update(item.id, { status: 'dismissed' }))}
                className="opacity-0 group-hover:opacity-100 text-gray-400 hover:text-red-600 p-1"
              >
                <Trash2 className="w-3.5 h-3.5" />
              </button>
            </li>
          ))}
        </ul>

        <form
          className="mt-3 flex gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            addAction();
          }}
        >
          <Input
            value={newAction}
            onChange={(e) => setNewAction(e.target.value)}
            placeholder="Add an action item the AI missed"
            className="h-8 text-sm"
          />
          <Button type="submit" size="sm" variant="outline" disabled={!newAction.trim()}>
            <Plus className="w-4 h-4" />
          </Button>
        </form>
      </div>

      {decisions.length > 0 && (
        <div>
          <h3 className="flex items-center gap-2 text-sm font-semibold text-gray-900 mb-3">
            <Gavel className="w-4 h-4" />
            Decisions
          </h3>
          <ul className="space-y-3">
            {decisions.map((item) => (
              <li key={item.id} className="group flex items-start gap-2">
                <span className="mt-1.5 w-1.5 h-1.5 rounded-full bg-gray-400 flex-shrink-0" />
                <div className="flex-1 min-w-0">
                  <p className="text-sm text-gray-900">{item.title}</p>
                  {item.description && <p className="text-xs text-gray-600 mt-0.5">{item.description}</p>}
                  {renderEvidence(item)}
                </div>
                <button
                  type="button"
                  aria-label="Remove decision"
                  onClick={() => (item.source === 'user' ? remove(item.id) : update(item.id, { status: 'dismissed' }))}
                  className="opacity-0 group-hover:opacity-100 text-gray-400 hover:text-red-600 p-1"
                >
                  <Trash2 className="w-3.5 h-3.5" />
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
    </section>
  );
}
