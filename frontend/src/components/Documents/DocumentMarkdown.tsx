'use client';

import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { renderToStaticMarkup } from 'react-dom/server';

const CLASSES =
  'prose prose-sm max-w-none prose-headings:font-semibold prose-h2:mt-6 prose-h2:mb-2 prose-table:text-sm prose-th:bg-gray-50 prose-td:align-top';

/** A document rendered for reading (and printing). */
export function DocumentMarkdown({ markdown, className = '' }: { markdown: string; className?: string }) {
  return (
    <div className={`${CLASSES} ${className}`}>
      <ReactMarkdown remarkPlugins={[remarkGfm]}>{markdown}</ReactMarkdown>
    </div>
  );
}

/** Standalone HTML for Word export (macOS textutil turns it into .docx). */
export function documentHtml(title: string, markdown: string): string {
  const body = renderToStaticMarkup(<ReactMarkdown remarkPlugins={[remarkGfm]}>{markdown}</ReactMarkdown>);
  const escapedTitle = title.replace(/&/g, '&amp;').replace(/</g, '&lt;');
  return `<!doctype html><html><head><meta charset="utf-8"><title>${escapedTitle}</title>
<style>
body { font-family: Calibri, Helvetica, Arial, sans-serif; font-size: 11pt; line-height: 1.4; }
h1 { font-size: 18pt; } h2 { font-size: 14pt; margin-top: 16pt; } h3 { font-size: 12pt; }
table { border-collapse: collapse; width: 100%; } th, td { border: 1px solid #999; padding: 4px 6px; vertical-align: top; }
th { background: #f2f2f2; }
</style></head><body><h1>${escapedTitle}</h1>${body}</body></html>`;
}
