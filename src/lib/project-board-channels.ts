import type { Channel, Snapshot } from './types';

const PREFIX = '__monitter-project-board__:';

export function projectBoardId(channelId: string): string | null {
  return channelId.startsWith(PREFIX) ? channelId.slice(PREFIX.length) : null;
}

/** UI-only channel projection. Board posts remain project-owned in Rust. */
export function projectBoardChannels(snapshot: Snapshot | null): Channel[] {
  if (!snapshot?.settings.projectBoardEnabled) return [];
  return snapshot.projects.map(project => ({
    id: `${PREFIX}${project.id}`,
    name: `#${project.name}`,
    description: `Private coordination board for ${project.name}`,
    agentIds: [],
    messages: (snapshot.projectBoardMessages ?? [])
      .filter(message => message.projectId === project.id)
      .map(message => ({
        id: message.id,
        role: message.agentId ? 'assistant' as const : 'user' as const,
        agentId: message.agentId,
        authorName: message.authorName,
        text: message.text,
        createdAt: message.createdAt,
        taskId: message.taskId,
      })),
  }));
}
