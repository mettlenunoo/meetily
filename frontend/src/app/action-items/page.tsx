'use client';

import { Suspense, useCallback, useEffect, useMemo, useState } from 'react';
import { useSearchParams } from 'next/navigation';
import { ListTodo, Search } from 'lucide-react';
import { Input } from '@/components/ui/input';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { ActionItemRow } from '@/components/ActionItems/ActionItemRow';
import { projectsApi, todayISO, type ActionItemFilter, type ProjectSummary, type TrackedItem } from '@/lib/projects';

type DueFilter = NonNullable<ActionItemFilter['due']>;
type StatusFilter = NonNullable<ActionItemFilter['status']>;

const DUE_OPTIONS: { value: DueFilter; label: string }[] = [
  { value: 'any', label: 'Any due date' },
  { value: 'overdue', label: 'Overdue' },
  { value: 'today', label: 'Due today' },
  { value: 'week', label: 'Due within 7 days' },
  { value: 'undated', label: 'No exact date' },
];

/** Every action item across all meetings, soonest due first. */
function ActionItems() {
  const ownerParam = useSearchParams().get('owner');
  const [items, setItems] = useState<TrackedItem[]>([]);
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [status, setStatus] = useState<StatusFilter>('open');
  const [due, setDue] = useState<DueFilter>('any');
  const [projectId, setProjectId] = useState<string>('all');
  const [owner, setOwner] = useState(ownerParam ?? '');
  // Exact name when opened from People; typing switches to "contains"
  const [ownerExact, setOwnerExact] = useState(!!ownerParam);

  const load = useCallback(async () => {
    try {
      const result = await projectsApi.actionItems({
        status,
        due,
        owner: owner.trim() || undefined,
        ownerExact,
        projectId: projectId === 'all' ? null : projectId,
      });
      setItems(result);
    } catch (error) {
      console.error('Failed to load action items:', error);
    } finally {
      setLoading(false);
    }
  }, [status, due, owner, ownerExact, projectId]);

  useEffect(() => {
    const t = setTimeout(load, owner ? 250 : 0); // debounce typing in the owner box
    return () => clearTimeout(t);
  }, [load, owner]);

  useEffect(() => {
    projectsApi.list({ includeArchived: true }).then(setProjects).catch(console.error);
  }, []);

  const onChanged = (id: string, changes: Partial<TrackedItem>) =>
    setItems((prev) => prev.map((it) => (it.id === id ? { ...it, ...changes } : it)));

  const counts = useMemo(() => {
    const today = todayISO();
    const open = items.filter((i) => i.status === 'open');
    return {
      open: open.length,
      overdue: open.filter((i) => i.dueAt && i.dueAt < today).length,
    };
  }, [items]);

  return (
    <div className="h-screen overflow-y-auto bg-gray-50">
      <div className="mx-auto max-w-4xl px-6 py-8">
        <header className="mb-6">
          <h1 className="flex items-center gap-2 text-2xl font-semibold text-gray-900">
            <ListTodo className="w-6 h-6" /> Action items
          </h1>
          <p className="mt-1 text-sm text-gray-500">
            From all your meetings. Dates come from what was said (“Friday” in a Wednesday meeting is that Friday); when
            the words aren’t one exact date, pick it yourself.
          </p>
        </header>

        <div className="mb-4 flex flex-wrap items-center gap-2">
          <Select value={status} onValueChange={(v) => setStatus(v as StatusFilter)}>
            <SelectTrigger className="w-32 bg-white"><SelectValue /></SelectTrigger>
            <SelectContent>
              <SelectItem value="open">Open</SelectItem>
              <SelectItem value="done">Done</SelectItem>
              <SelectItem value="all">All</SelectItem>
            </SelectContent>
          </Select>
          <Select value={due} onValueChange={(v) => setDue(v as DueFilter)}>
            <SelectTrigger className="w-44 bg-white"><SelectValue /></SelectTrigger>
            <SelectContent>
              {DUE_OPTIONS.map((o) => <SelectItem key={o.value} value={o.value}>{o.label}</SelectItem>)}
            </SelectContent>
          </Select>
          <Select value={projectId} onValueChange={setProjectId}>
            <SelectTrigger className="w-48 bg-white"><SelectValue /></SelectTrigger>
            <SelectContent>
              <SelectItem value="all">All projects</SelectItem>
              <SelectItem value="none">No project</SelectItem>
              {projects.map((p) => <SelectItem key={p.id} value={p.id}>{p.name}</SelectItem>)}
            </SelectContent>
          </Select>
          <div className="relative">
            <Search className="pointer-events-none absolute left-2 top-2.5 w-4 h-4 text-gray-400" />
            <Input value={owner} onChange={(e) => { setOwner(e.target.value); setOwnerExact(false); }} placeholder="Owner" className="w-40 bg-white pl-8" />
          </div>
          {status !== 'done' && (
            <span className="ml-auto text-sm text-gray-500">
              {counts.open} open{counts.overdue > 0 && <span className="text-red-600">, {counts.overdue} overdue</span>}
            </span>
          )}
        </div>

        <div className="rounded-lg border border-gray-200 bg-white px-4">
          {loading ? (
            <p className="py-8 text-center text-sm text-gray-500">Loading…</p>
          ) : items.length === 0 ? (
            <p className="py-8 text-center text-sm text-gray-500">
              Nothing here. Action items appear after a meeting is summarised, or when you add them in a meeting.
            </p>
          ) : (
            <ul className="divide-y divide-gray-100">
              {items.map((item) => (
                <ActionItemRow key={item.id} item={item} onChanged={onChanged} />
              ))}
            </ul>
          )}
        </div>
      </div>
    </div>
  );
}

export default function ActionItemsPage() {
  return (
    <Suspense fallback={null}>
      <ActionItems />
    </Suspense>
  );
}
