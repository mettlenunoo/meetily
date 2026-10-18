'use client';

import { useEffect, useState } from 'react';
import { toast } from 'sonner';
import { Lock } from 'lucide-react';
import { Dialog, DialogContent, DialogFooter, DialogTitle } from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Textarea } from '@/components/ui/textarea';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { KIND_LABELS, projectsApi, type Project, type ProjectKind, type ProjectPrivacy } from '@/lib/projects';

interface ProjectDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Edit this project; create a new one when absent */
  project?: Project | null;
  /** Kinds offered in the picker (the Study area offers only 'course') */
  kinds?: ProjectKind[];
  onSaved: (project: Project) => void;
}

interface ProjectForm {
  name: string;
  kind: ProjectKind;
  description: string;
  people: string;
  keywords: string;
  slackChannels: string;
  privacy: ProjectPrivacy;
}

const EMPTY: ProjectForm = {
  name: '',
  kind: 'work',
  description: '',
  people: '',
  keywords: '',
  slackChannels: '',
  privacy: 'claude',
};

export function ProjectDialog({ open, onOpenChange, project, kinds = ['work', 'client', 'personal'], onSaved }: ProjectDialogProps) {
  const [form, setForm] = useState<ProjectForm>(EMPTY);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!open) return;
    setForm(
      project
        ? {
            name: project.name,
            kind: project.kind,
            description: project.description ?? '',
            people: project.people ?? '',
            keywords: project.keywords ?? '',
            slackChannels: project.slackChannels ?? '',
            privacy: project.privacy,
          }
        : { ...EMPTY, kind: kinds[0] },
    );
  }, [open, project, kinds]);

  const set = <K extends keyof ProjectForm>(key: K, value: ProjectForm[K]) => setForm((f) => ({ ...f, [key]: value }));

  const save = async () => {
    if (!form.name.trim()) {
      toast.error('Give the project a name');
      return;
    }
    setSaving(true);
    try {
      const saved = project ? await projectsApi.update(project.id, form) : await projectsApi.create(form);
      toast.success(project ? 'Project saved' : `Created ${saved.name}`);
      onSaved(saved);
      onOpenChange(false);
    } catch (error) {
      toast.error(String(error));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[520px]">
        <DialogTitle>{project ? `Edit ${project.name}` : form.kind === 'course' ? 'New course' : 'New project'}</DialogTitle>
        <div className="space-y-4 py-2">
          <div className="grid grid-cols-[1fr_140px] gap-3">
            <div className="space-y-1.5">
              <Label htmlFor="project-name">Name</Label>
              <Input
                id="project-name"
                autoFocus
                value={form.name}
                onChange={(e) => set('name', e.target.value)}
                placeholder={form.kind === 'course' ? 'e.g. GEOG 301 Remote Sensing' : 'e.g. IFDC Geoportal'}
              />
            </div>
            {kinds.length > 1 && (
              <div className="space-y-1.5">
                <Label>Type</Label>
                <Select value={form.kind} onValueChange={(v) => set('kind', v as ProjectKind)}>
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {kinds.map((k) => (
                      <SelectItem key={k} value={k}>
                        {KIND_LABELS[k]}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            )}
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="project-description">Description</Label>
            <Textarea
              id="project-description"
              rows={2}
              value={form.description}
              onChange={(e) => set('description', e.target.value)}
              placeholder="What it's about, in a sentence. Helps Hearken file meetings correctly."
            />
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-1.5">
              <Label htmlFor="project-people">People</Label>
              <Input
                id="project-people"
                value={form.people}
                onChange={(e) => set('people', e.target.value)}
                placeholder="Kofi Mensah, Ama Owusu"
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="project-keywords">Keywords</Label>
              <Input
                id="project-keywords"
                value={form.keywords}
                onChange={(e) => set('keywords', e.target.value)}
                placeholder="GIS, map layers, basemap"
              />
            </div>
          </div>
          <p className="-mt-2 text-xs text-gray-500">
            Comma-separated. When these come up in a recording, Hearken suggests this project. You always confirm.
          </p>

          {form.kind !== 'course' && (
            <div className="space-y-1.5">
              <Label htmlFor="project-slack">Slack channels</Label>
              <Input
                id="project-slack"
                value={form.slackChannels}
                onChange={(e) => set('slackChannels', e.target.value)}
                placeholder="#ifdc-geoportal"
              />
            </div>
          )}

          <div className="flex items-start justify-between gap-4 rounded-md border border-gray-200 p-3">
            <div>
              <p className="flex items-center gap-1.5 text-sm font-medium text-gray-900">
                <Lock className="w-3.5 h-3.5" /> Local only
              </p>
              <p className="text-xs text-gray-500 mt-0.5">
                Hide this {form.kind === 'course' ? 'course' : 'project'} and its recordings from Claude (the Hearken connector),
                and never send them to a cloud summary model.
              </p>
            </div>
            <Switch checked={form.privacy === 'local'} onCheckedChange={(on) => set('privacy', on ? 'local' : 'claude')} />
          </div>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button onClick={save} disabled={saving}>
            {project ? 'Save' : 'Create'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
