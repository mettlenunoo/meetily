'use client';

import { useState } from 'react';
import { CalendarClock, X } from 'lucide-react';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { DUE_STYLES, dueState, formatDue } from '@/lib/projects';
import type { MeetingItem } from '@/components/MeetingDetails/MeetingItemsPanel';

interface DueDateButtonProps {
  item: Pick<MeetingItem, 'dueAt' | 'dueDate' | 'status'>;
  /** YYYY-MM-DD, or '' to clear */
  onChange: (dueAt: string) => void;
}

/**
 * Shows an item's due date: the exact date when known (red when overdue, amber today,
 * purple within a week) plus the words as spoken, e.g. "Fri 16 Oct · Friday". When the
 * spoken words couldn't be turned into one date, they show with a dashed border so the
 * user can pick the date.
 */
export function DueDateButton({ item, onChange }: DueDateButtonProps) {
  const [open, setOpen] = useState(false);
  const state = dueState(item);
  const spoken = item.dueDate && item.dueDate !== item.dueAt ? item.dueDate : null;

  let label: string;
  if (item.dueAt) {
    label = formatDue(item.dueAt) + (spoken ? ` · ${spoken}` : '');
  } else if (spoken) {
    label = `${spoken} (pick a date)`;
  } else {
    label = 'No due date';
  }
  const title =
    state === 'overdue' ? 'Overdue' : !item.dueAt && spoken ? `Said "${spoken}", which isn't one exact date` : undefined;

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          title={title}
          className={`inline-flex items-center gap-1 rounded px-1.5 py-0.5 border text-xs ${
            !item.dueAt && spoken ? 'border-dashed border-amber-300 text-amber-700' : DUE_STYLES[state]
          }`}
        >
          <CalendarClock className="w-3 h-3" />
          {state === 'overdue' && <span className="font-medium">Overdue:</span>}
          {label}
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-auto p-3" align="start">
        <div className="flex items-center gap-2">
          <input
            type="date"
            autoFocus
            defaultValue={item.dueAt ?? ''}
            onChange={(e) => {
              if (e.target.value) {
                onChange(e.target.value);
                setOpen(false);
              }
            }}
            className="h-8 rounded border border-gray-300 px-2 text-sm"
          />
          {item.dueAt && (
            <button
              type="button"
              onClick={() => {
                onChange('');
                setOpen(false);
              }}
              className="inline-flex items-center gap-1 text-xs text-gray-500 hover:text-red-600"
            >
              <X className="w-3 h-3" /> Clear
            </button>
          )}
        </div>
        {spoken && <p className="mt-2 text-xs text-gray-500">Said in the meeting: “{spoken}”</p>}
      </PopoverContent>
    </Popover>
  );
}
