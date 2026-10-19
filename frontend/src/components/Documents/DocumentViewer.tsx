'use client';

import { useEffect, useState } from 'react';
import { toast } from 'sonner';
import { Download, FileType, Pencil, Printer, Trash2 } from 'lucide-react';
import { Dialog, DialogContent, DialogTitle } from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Textarea } from '@/components/ui/textarea';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu';
import { ConfirmationModal } from '@/components/ConfirmationModel/confirmation-modal';
import { DocumentMarkdown, documentHtml } from './DocumentMarkdown';
import { describeAuthor, documentsApi, type HearkenDocument } from '@/lib/documents';

interface DocumentViewerProps {
  documentId: string | null;
  onClose: () => void;
  onChanged: () => void;
}

/** Read, edit, export (Markdown / Word / PDF) or delete one document. */
export function DocumentViewer({ documentId, onClose, onChanged }: DocumentViewerProps) {
  const [doc, setDoc] = useState<HearkenDocument | null>(null);
  const [editing, setEditing] = useState(false);
  const [draftTitle, setDraftTitle] = useState('');
  const [draft, setDraft] = useState('');
  const [confirmDelete, setConfirmDelete] = useState(false);

  useEffect(() => {
    setEditing(false);
    if (!documentId) {
      setDoc(null);
      return;
    }
    documentsApi.get(documentId).then(setDoc).catch((e) => toast.error(String(e)));
  }, [documentId]);

  const startEdit = () => {
    if (!doc) return;
    setDraftTitle(doc.title);
    setDraft(doc.markdown);
    setEditing(true);
  };

  const save = async () => {
    if (!doc) return;
    try {
      setDoc(await documentsApi.update(doc.id, { title: draftTitle, markdown: draft }));
      setEditing(false);
      onChanged();
      toast.success('Saved');
    } catch (e) {
      toast.error(String(e));
    }
  };

  const exportAs = async (format: 'md' | 'docx') => {
    if (!doc) return;
    try {
      const path = await documentsApi.export(doc.id, format, format === 'docx' ? documentHtml(doc.title, doc.markdown) : undefined);
      if (path) toast.success('Exported', { description: path });
    } catch (e) {
      toast.error(String(e));
    }
  };

  const remove = async () => {
    if (!doc) return;
    setConfirmDelete(false);
    try {
      await documentsApi.remove(doc.id);
      onChanged();
      onClose();
    } catch (e) {
      toast.error(String(e));
    }
  };

  return (
    <Dialog open={!!documentId} onOpenChange={(o) => { if (!o) onClose(); }}>
      <DialogContent className="flex max-h-[90vh] max-w-3xl flex-col gap-0 p-0">
        <div className="flex items-center gap-2 border-b border-gray-200 px-5 py-3 pr-12">
          {editing ? (
            <Input value={draftTitle} onChange={(e) => setDraftTitle(e.target.value)} className="h-8 flex-1" aria-label="Title" />
          ) : (
            <DialogTitle className="min-w-0 flex-1 truncate text-base">{doc?.title ?? 'Document'}</DialogTitle>
          )}
          {doc && doc.status !== 'generating' && !editing && (
            <>
              <Button variant="outline" size="sm" className="h-8" onClick={startEdit}>
                <Pencil className="w-3.5 h-3.5 mr-1" /> Edit
              </Button>
              {doc.status === 'ready' && (
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <Button variant="outline" size="sm" className="h-8">
                      <Download className="w-3.5 h-3.5 mr-1" /> Export
                    </Button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end">
                    <DropdownMenuItem onClick={() => exportAs('docx')}>
                      <FileType className="w-4 h-4 mr-2" /> Word (.docx)
                    </DropdownMenuItem>
                    <DropdownMenuItem onClick={() => documentsApi.print(doc.id).catch((e) => toast.error(String(e)))}>
                      <Printer className="w-4 h-4 mr-2" /> PDF / Print…
                    </DropdownMenuItem>
                    <DropdownMenuItem onClick={() => exportAs('md')}>
                      <FileType className="w-4 h-4 mr-2" /> Markdown (.md)
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
              )}
              <Button variant="outline" size="icon" className="h-8 w-8" aria-label="Delete document" onClick={() => setConfirmDelete(true)}>
                <Trash2 className="w-3.5 h-3.5" />
              </Button>
            </>
          )}
          {editing && (
            <>
              <Button variant="outline" size="sm" className="h-8" onClick={() => setEditing(false)}>Cancel</Button>
              <Button size="sm" className="h-8" onClick={save}>Save</Button>
            </>
          )}
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">
          {!doc ? (
            <p className="text-sm text-gray-500">Loading…</p>
          ) : doc.status === 'failed' ? (
            <div className="rounded-md border border-amber-200 bg-amber-50 p-3 text-sm text-amber-900">
              This document could not be generated: {doc.error ?? 'unknown error'}. Delete it and try again, or write it yourself with Edit.
            </div>
          ) : editing ? (
            <Textarea value={draft} onChange={(e) => setDraft(e.target.value)} className="min-h-[60vh] font-mono text-sm" aria-label="Document text (Markdown)" />
          ) : (
            <DocumentMarkdown markdown={doc.markdown} />
          )}
        </div>
        {doc && doc.status === 'ready' && !editing && (
          <p className="border-t border-gray-100 px-5 py-2 text-xs text-gray-500">
            {doc.userEdited ? 'Edited by you' : `Written by ${describeAuthor(doc.generatedBy)}`}. Check names, numbers and dates against the recording before sharing.
          </p>
        )}
      </DialogContent>
      <ConfirmationModal
        isOpen={confirmDelete}
        text={`Delete "${doc?.title}"? This can't be undone. The recording, summary and items are not affected.`}
        onConfirm={remove}
        onCancel={() => setConfirmDelete(false)}
      />
    </Dialog>
  );
}
