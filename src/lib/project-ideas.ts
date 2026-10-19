import type { Project, ProjectIdea, ProjectIdeaStatus } from './types';

export interface IdeaDraft { idea: ProjectIdea; revision: number; }
export const ideaStatuses: ProjectIdeaStatus[] = ['open', 'inPlan', 'done', 'dropped'];
export function newIdeaDraft(project: Project): IdeaDraft {
  return { revision: project.ideasRevision ?? 0, idea: { id: crypto.randomUUID(), title: '', markdown: '', createdAt: 0, updatedAt: 0, status: 'open', tags: [] } };
}
export function editIdeaDraft(project: Project, idea: ProjectIdea): IdeaDraft {
  // These records use the JSON wire format. Copy through it so reactive Svelte
  // proxies are accepted while nested tags and history remain draft-local.
  return { revision: project.ideasRevision ?? 0, idea: JSON.parse(JSON.stringify(idea)) as ProjectIdea };
}
export function ideaSaveList(project: Project, draft: IdeaDraft): ProjectIdea[] {
  if ((project.ideasRevision ?? 0) !== draft.revision) throw new Error('The Idea List changed elsewhere. Your draft is retained. Review the latest list before saving.');
  const title = draft.idea.title.trim();
  if (!title || [...title].length > 240) throw new Error('Use a title of 1–240 characters.');
  if (new TextEncoder().encode(draft.idea.markdown).length > 128 * 1024) throw new Error('Idea Markdown is limited to 128 KiB.');
  const idea = { ...draft.idea, title };
  const existing = project.ideas ?? [];
  return existing.some(item => item.id === idea.id) ? existing.map(item => item.id === idea.id ? idea : item) : [...existing, idea];
}
export function parseIdeaTags(text: string): string[] {
  const tags = [...new Set(text.split(',').map(tag => tag.trim()).filter(Boolean))];
  if (tags.length > 16 || tags.some(tag => [...tag].length > 32)) throw new Error('Use up to 16 tags of 1–32 characters.');
  return tags;
}
