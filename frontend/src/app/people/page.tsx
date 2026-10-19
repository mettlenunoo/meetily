'use client';

import { useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { AlertCircle, Users } from 'lucide-react';

interface Person {
  name: string;
  open: number;
  overdue: number;
  done: number;
  meetings: number;
  lastSeen: string | null;
  projects: string[];
}

/** Everyone named as an action-item owner: what they've committed to, and what's late. */
export default function PeoplePage() {
  const router = useRouter();
  const [people, setPeople] = useState<Person[] | null>(null);

  useEffect(() => {
    invoke<Person[]>('api_list_people').then(setPeople).catch(console.error);
  }, []);

  return (
    <div className="h-screen overflow-y-auto bg-gray-50">
      <div className="mx-auto max-w-4xl px-6 py-8">
        <h1 className="flex items-center gap-2 text-2xl font-semibold text-gray-900">
          <Users className="w-6 h-6" /> People
        </h1>
        <p className="mt-1 mb-6 text-sm text-gray-500">
          Everyone named as the owner of an action item. Names are shown as they were said; Hearken never merges people
          by guessing. “Me” is you, from the speaker labels.
        </p>
        {!people ? (
          <p className="text-sm text-gray-500">Loading…</p>
        ) : people.length === 0 ? (
          <p className="rounded-lg border border-dashed border-gray-300 bg-white p-8 text-center text-sm text-gray-500">
            Nobody yet. People appear here when action items have an owner.
          </p>
        ) : (
          <ul className="divide-y divide-gray-100 rounded-lg border border-gray-200 bg-white">
            {people.map((p) => (
              <li key={p.name}>
                <button
                  type="button"
                  onClick={() => router.push(`/action-items?owner=${encodeURIComponent(p.name)}`)}
                  className="flex w-full items-center gap-4 px-4 py-3 text-left hover:bg-gray-50"
                >
                  <span className="flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-full bg-gray-100 text-sm font-medium text-gray-700">
                    {p.name === 'Me' ? 'You' : p.name.split(/\s+/).map((w) => w[0]).slice(0, 2).join('').toUpperCase()}
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block font-medium text-gray-900">{p.name === 'Me' ? 'You' : p.name}</span>
                    <span className="block truncate text-xs text-gray-500">
                      {p.meetings} meeting{p.meetings === 1 ? '' : 's'}
                      {p.projects.length > 0 && ` · ${p.projects.join(', ')}`}
                    </span>
                  </span>
                  <span className="flex-shrink-0 text-right text-sm">
                    <span className="text-gray-900">{p.open} open</span>
                    {p.overdue > 0 && (
                      <span className="ml-2 inline-flex items-center gap-1 text-red-600">
                        <AlertCircle className="w-3.5 h-3.5" /> {p.overdue} overdue
                      </span>
                    )}
                    <span className="block text-xs text-gray-400">{p.done} done</span>
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
