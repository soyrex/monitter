import type { Agent, Collaboration, Message, RunEvent, Snapshot, SubagentSession, SubagentTranscriptEntry, Task, TaskStatus } from '$lib/types';
import { subagentThreadLink, toolPresentation } from '$lib/activity-grouping';

/**
 * A presentation-only task view. Both native child tasks and Monitter
 * delegations are deliberately normalised here so no source label leaks into
 * the UI.
 */
export interface UnifiedSubagent {
  id: string;
  taskId: string | null;
  collaborationId: string | null;
  agentThreadId: string | null;
  title: string;
  agentName: string;
  agentInitials: string;
  status: TaskStatus | 'queued';
  summary: string | null;
  activity: string | null;
  prompt: string | null;
  result: string | null;
  error: string | null;
  model: string | null;
  reasoningEffort: string | null;
  transcript: SubagentTranscriptEntry[];
  updatedAt: number;
}

export interface UnifiedSubagentInput {
  tasks: Task[];
  agents: Agent[];
  collaborations: Collaboration[];
  messages?: Message[];
  events?: RunEvent[];
  subagentSessions?: SubagentSession[];
  parentTaskId: string;
}

export const activeUnifiedSubagent = (item: UnifiedSubagent) =>
  item.status === 'queued' || item.status === 'running';

export const unifiedSubagentStatus = (status: UnifiedSubagent['status']) => ({
  queued: 'Starting',
  running: 'Working',
  completed: 'Done',
  error: 'Needs attention',
  interrupted: 'Stopped',
  idle: 'Waiting',
})[status];

export function unifiedSubagentDomId(prefix: string, id: string) {
  return `${prefix}-${id}`.replace(/[^a-zA-Z0-9_-]/g, '-');
}

export function unifiedSubagentInitials(name: string) {
  const initials = name.trim().split(/\s+/).filter(Boolean).slice(0, 2).map(part => part[0]?.toUpperCase()).join('');
  return initials || 'S';
}

function humanName(path: string | null | undefined) {
  const leaf = path?.split('/').filter(Boolean).at(-1) ?? '';
  const value = leaf.replace(/^agent[-_]?/i, '').replace(/[-_]+/g, ' ').replace(/\b\w/g, letter => letter.toUpperCase()).trim();
  return value || 'Subagent';
}

function concise(value: string | null | undefined, fallback: string) {
  const line = value?.trim().split(/\r?\n/, 1)[0]?.replace(/\s+/g, ' ') ?? '';
  return line ? (line.length > 74 ? `${line.slice(0, 71).trimEnd()}…` : line) : fallback;
}

function sessionTree(
  sessions: SubagentSession[],
  collaborations: Collaboration[],
  parentTaskId: string,
) {
  const byParent = new Map<string, SubagentSession[]>();
  for (const session of sessions) {
    const list = byParent.get(session.parentTaskId) ?? [];
    list.push(session);
    byParent.set(session.parentTaskId, list);
  }
  const collaborationById = new Map(collaborations.map(value => [value.id, value]));
  const result: SubagentSession[] = [];
  const seenSessions = new Set<string>();
  const seenTasks = new Set<string>([parentTaskId]);
  const pending = [parentTaskId];
  while (pending.length) {
    const taskId = pending.shift()!;
    for (const session of byParent.get(taskId) ?? []) {
      if (!seenSessions.has(session.id)) {
        seenSessions.add(session.id);
        result.push(session);
      }
      const childTaskId = session.collaborationId ? collaborationById.get(session.collaborationId)?.toTaskId : null;
      if (childTaskId && !seenTasks.has(childTaskId)) {
        seenTasks.add(childTaskId);
        pending.push(childTaskId);
      }
    }
  }
  return result;
}

/**
 * Produces an ordered, deduplicated list from the backend's real task and
 * collaboration records. A task is authoritative when it exists; a
 * collaboration-only record remains visible rather than becoming a fake task.
 */
export function unifiedSubagentsForTask({ tasks, agents, collaborations, messages = [], events = [], subagentSessions = [], parentTaskId }: UnifiedSubagentInput): UnifiedSubagent[] {
  const agentById = new Map(agents.map(agent => [agent.id, agent]));
  const taskById = new Map(tasks.map(task => [task.id, task]));
  const delegationById = new Map(collaborations.filter(value => value.kind === 'delegation').map(value => [value.id, value]));
  const sessions = [...subagentSessions];
  const representedCollaborations = new Set(sessions.flatMap(value => value.collaborationId ? [value.collaborationId] : []));
  // Compatibility for a desktop that has not yet emitted the normalized field.
  for (const collaboration of collaborations) {
    if (collaboration.kind !== 'delegation' || representedCollaborations.has(collaboration.id)) continue;
    sessions.push({
      id: `collaboration:${collaboration.id}`, source: 'collaboration', parentTaskId: collaboration.fromTaskId,
      collaborationId: collaboration.id, prompt: collaboration.text, status: collaboration.status,
      result: collaboration.result, error: collaboration.error, createdAt: collaboration.createdAt, updatedAt: collaboration.updatedAt,
    });
  }

  return sessionTree(sessions, collaborations, parentTaskId).map(session => {
    const collaboration = session.collaborationId ? delegationById.get(session.collaborationId) : undefined;
    const task = collaboration ? taskById.get(collaboration.toTaskId) : undefined;
    const agent = agentById.get(task?.agentId ?? collaboration?.toAgentId ?? '');
    const matchingEvents = events.filter(event => {
      if (event.kind !== 'subagent' || event.taskId !== session.parentTaskId) return false;
      const activity = subagentThreadLink(event);
      return !!activity && [activity.agentThreadId, ...activity.receiverThreadIds].includes(session.agentThreadId ?? '');
    });
    const latestEvent = matchingEvents.sort((a, b) => b.createdAt - a.createdAt)[0]
      ?? (task ? events.filter(event => event.taskId === task.id).sort((a, b) => b.createdAt - a.createdAt)[0] : undefined);
    const latestActivity = latestEvent ? subagentThreadLink(latestEvent) : null;
    const agentName = agent?.name ?? humanName(session.agentPath);
    const prompt = session.prompt ?? collaboration?.text ?? null;
    const result = session.result ?? collaboration?.result ?? null;
    const error = session.error ?? collaboration?.error ?? null;
    const title = task?.title || concise(prompt, agentName);

    return {
      id: session.id,
      taskId: task?.id ?? null,
      collaborationId: session.collaborationId ?? null,
      agentThreadId: session.agentThreadId ?? null,
      title,
      agentName,
      agentInitials: unifiedSubagentInitials(agentName),
      status: session.status,
      summary: error || result || prompt,
      activity: latestActivity && latestEvent ? toolPresentation(latestEvent, session.status === 'running').label : latestEvent?.title || null,
      prompt,
      result,
      error,
      model: session.model ?? task?.model ?? null,
      reasoningEffort: session.reasoningEffort ?? task?.modelSettings?.reasoningEffort ?? null,
      transcript: task ? messages
        .filter(message => message.taskId === task.id && ['user', 'assistant'].includes(message.role))
        .map(message => ({
          id: message.id,
          role: message.role as 'user' | 'assistant',
          text: message.text,
          createdAt: message.createdAt,
        })) : [],
      updatedAt: Math.max(session.updatedAt, task?.updatedAt ?? 0, collaboration?.updatedAt ?? 0, latestEvent?.createdAt ?? 0),
    };
  }).sort((a, b) => {
    const activeOrder = Number(activeUnifiedSubagent(b)) - Number(activeUnifiedSubagent(a));
    return activeOrder || b.updatedAt - a.updatedAt || a.title.localeCompare(b.title);
  });
}

/** Convenience adapter for the regular desktop snapshot. */
export function unifiedSubagentsFromSnapshot(snapshot: Pick<Snapshot, 'tasks' | 'agents' | 'collaborations' | 'messages' | 'events' | 'subagentSessions'>, parentTaskId: string) {
  return unifiedSubagentsForTask({ ...snapshot, subagentSessions: snapshot.subagentSessions ?? [], parentTaskId });
}
