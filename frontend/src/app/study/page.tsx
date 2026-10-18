'use client';

import { ProjectsList } from '@/components/Projects/ProjectsList';

/** The Study area: courses (stored as projects with kind 'course') and their lectures. */
export default function StudyPage() {
  return <ProjectsList mode="study" />;
}
