import type { Agent, Collaboration, Snapshot, SubagentSession, Task } from '$lib/types';

export interface SidebarChildAgent {
  id: string;
  parentTaskId: string;
  task: Task | null;
  sessionId: string | null;
  title: string;
  agentName: string;
  status: SubagentSession['status'] | Task['status'];
  updatedAt: number;
}

function childName(path: string | null | undefined) {
  const leaf = path?.split('/').filter(Boolean).at(-1) ?? '';
  return leaf.replace(/^agent[-_]?/i, '').replace(/[-_]+/g, ' ').replace(/\b\w/g, letter => letter.toUpperCase()).trim() || 'Subagent';
}

function childTitle(task: Task | null, prompt: string | null | undefined, fallback: string) {
  const firstLine = prompt?.trim().split(/\r?\n/, 1)[0]?.replace(/\s+/g, ' ') ?? '';
  return task?.title || (firstLine.length > 72 ? `${firstLine.slice(0, 69).trimEnd()}…` : firstLine) || fallback;
}

/** One direct child row per native session, delegation, or unrepresented child task. */
export function sidebarChildAgents(
  snapshot: Pick<Snapshot, 'agents' | 'collaborations' | 'subagentSessions'>,
  visibleTasks: readonly Task[],
): ReadonlyMap<string, SidebarChildAgent[]> {
  const tasks = new Map(visibleTasks.map(task => [task.id, task]));
  const agents = new Map(snapshot.agents.map(agent => [agent.id, agent]));
  const collaborations = new Map(snapshot.collaborations.filter(item => item.kind === 'delegation').map(item => [item.id, item]));
  const rows = new Map<string, SidebarChildAgent[]>();
  const representedTasks = new Set<string>();
  const representedCollaborations = new Set<string>();
  const add = (row: SidebarChildAgent) => {
    if (!tasks.has(row.parentTaskId) || row.task?.id === row.parentTaskId) return;
    const siblings = rows.get(row.parentTaskId) ?? [];
    siblings.push(row);
    rows.set(row.parentTaskId, siblings);
  };
  const agentName = (agent: Agent | undefined, session?: SubagentSession) => agent?.name ?? childName(session?.agentPath);
  const taskFor = (collaboration: Collaboration | undefined) => collaboration ? tasks.get(collaboration.toTaskId) ?? null : null;

  for (const session of snapshot.subagentSessions ?? []) {
    const collaboration = session.collaborationId ? collaborations.get(session.collaborationId) : undefined;
    const task = taskFor(collaboration);
    const name = agentName(agents.get(task?.agentId ?? collaboration?.toAgentId ?? ''), session);
    add({
      id: session.id, parentTaskId: session.parentTaskId, task, sessionId: session.id,
      title: childTitle(task, session.prompt ?? collaboration?.text, name), agentName: name,
      status: session.status, updatedAt: Math.max(session.updatedAt, task?.updatedAt ?? 0),
    });
    if (task) representedTasks.add(task.id);
    if (collaboration) representedCollaborations.add(collaboration.id);
  }

  // Older desktop builds may have delegations but not the normalized session array.
  for (const collaboration of collaborations.values()) {
    if (representedCollaborations.has(collaboration.id)) continue;
    const task = taskFor(collaboration);
    const name = agentName(agents.get(task?.agentId ?? collaboration.toAgentId));
    add({
      id: `collaboration:${collaboration.id}`, parentTaskId: collaboration.fromTaskId,
      task, sessionId: `collaboration:${collaboration.id}`, title: childTitle(task, collaboration.text, name),
      agentName: name, status: collaboration.status, updatedAt: Math.max(collaboration.updatedAt, task?.updatedAt ?? 0),
    });
    if (task) representedTasks.add(task.id);
  }

  for (const task of visibleTasks) {
    if (!task.parentTaskId || representedTasks.has(task.id)) continue;
    add({
      id: `task:${task.id}`, parentTaskId: task.parentTaskId, task, sessionId: null,
      title: task.title, agentName: agentName(agents.get(task.agentId)),
      status: task.status, updatedAt: task.updatedAt,
    });
  }

  for (const siblings of rows.values()) siblings.sort((a, b) =>
    Number(b.status === 'running' || b.status === 'queued') - Number(a.status === 'running' || a.status === 'queued')
      || b.updatedAt - a.updatedAt || a.id.localeCompare(b.id));
  return rows;
}

/** A child with a visible parent belongs only beneath that parent, never twice at the top level. */
export function sidebarRootTasks(tasks: readonly Task[]): Task[] {
  const visibleIds = new Set(tasks.map(task => task.id));
  return tasks.filter(task => !task.parentTaskId || !visibleIds.has(task.parentTaskId));
}
