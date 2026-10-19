'use client';

import { Suspense, useEffect, useState } from 'react';
import { useSearchParams } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { DocumentMarkdown } from '@/components/Documents/DocumentMarkdown';
import { documentsApi, type HearkenDocument } from '@/lib/documents';

/**
 * A document laid out for paper, in its own window. Once rendered it opens the macOS
 * print dialog, where "Save as PDF" makes the PDF.
 */
function PrintContent() {
  const id = useSearchParams().get('id');
  const [doc, setDoc] = useState<HearkenDocument | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!id) return;
    documentsApi.get(id).then(setDoc).catch((e) => setError(String(e)));
  }, [id]);

  useEffect(() => {
    if (!doc) return;
    document.title = doc.title;
    // Let the layout settle before printing
    const t = setTimeout(() => invoke('api_print_window').catch((e) => setError(String(e))), 400);
    return () => clearTimeout(t);
  }, [doc]);

  if (error) return <p className="p-8 text-sm text-red-600">{error}</p>;
  if (!doc) return <p className="p-8 text-sm text-gray-500">Preparing…</p>;

  return (
    <div className="print-document mx-auto max-w-[720px] bg-white px-10 py-10 text-gray-900">
      <style>{`
        @media print {
          @page { margin: 18mm; }
          .print-hint { display: none; }
          body { background: white; }
        }
      `}</style>
      <p className="print-hint mb-6 rounded bg-blue-50 px-3 py-2 text-xs text-blue-800">
        In the print dialog, open the PDF menu and choose “Save as PDF”. You can close this window afterwards.{' '}
        <button type="button" className="underline" onClick={() => invoke('api_print_window')}>Print again</button>
      </p>
      <h1 className="mb-1 text-2xl font-semibold">{doc.title}</h1>
      <p className="mb-6 text-xs text-gray-500">
        {new Date(doc.updatedAt).toLocaleDateString(undefined, { day: 'numeric', month: 'long', year: 'numeric' })}
      </p>
      <DocumentMarkdown markdown={doc.markdown} />
    </div>
  );
}

export default function PrintPage() {
  return (
    <Suspense fallback={null}>
      <PrintContent />
    </Suspense>
  );
}
