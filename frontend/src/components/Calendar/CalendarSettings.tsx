'use client';

import { useEffect, useState } from 'react';
import { toast } from 'sonner';
import { CalendarDays } from 'lucide-react';
import { Switch } from '@/components/ui/switch';
import { calendarApi, type CalendarStatus } from '@/lib/calendar';

/** Settings → Calendar: read the Mac's Calendar to name recordings and remind you to record. */
export function CalendarSettingsPanel() {
  const [status, setStatus] = useState<CalendarStatus | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => { calendarApi.status().then(setStatus).catch((e) => toast.error(String(e))); }, []);
  if (!status) return <p className="text-sm text-gray-500">Loading…</p>;

  const save = async (changes: Partial<CalendarStatus['settings']>) => {
    setBusy(true);
    try {
      setStatus(await calendarApi.set({ ...status.settings, ...changes }));
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
          <CalendarDays className="w-5 h-5" /> Calendar
        </h3>
        <p className="mt-1 text-sm text-gray-600">
          Hearken can read the calendar on this Mac (including synced Google or work calendars) to name each recording after
          the meeting it happened in, remember who was invited, use them to suggest the project, and remind you to record
          when a meeting with other people starts. It only reads; nothing is sent anywhere.
        </p>
      </div>
      <div className="flex items-center justify-between rounded-md border border-gray-200 p-3">
        <div>
          <p className="text-sm font-medium text-gray-900">Use my calendar</p>
          <p className="text-xs text-gray-500">
            {status.access === 'granted' ? 'macOS access granted.' : status.access === 'denied'
              ? 'Access was refused. Allow Hearken in System Settings → Privacy & Security → Calendars.'
              : 'macOS will ask for permission the first time.'}
          </p>
        </div>
        <Switch checked={status.settings.enabled} disabled={busy} onCheckedChange={(on) => save({ enabled: on })} />
      </div>
      {status.settings.enabled && (
        <div className="flex items-center justify-between rounded-md border border-gray-200 p-3">
          <div>
            <p className="text-sm font-medium text-gray-900">Remind me to record</p>
            <p className="text-xs text-gray-500">A notification when an event with other people starts (once per event).</p>
          </div>
          <Switch checked={status.settings.reminders} disabled={busy} onCheckedChange={(on) => save({ reminders: on })} />
        </div>
      )}
    </div>
  );
}
