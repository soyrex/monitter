import { writable } from 'svelte/store';
import type { Agent, Message, Project, Snapshot, Task } from '$lib/types';

export type OperatorRole = 'primary user' | 'visitor';
export interface OperatorIdentity { name: string; role: OperatorRole; }
export interface ActiveOperatorShare {
  primary: OperatorIdentity;
  visitor: OperatorIdentity;
  taskIds: string[];
  projectIds: string[];
}

export const activeOperatorShare = writable<ActiveOperatorShare | null>(null);

const operatorPrefix = /^@\(([^\n()]{1,48})\):\s*/;

export function validOperatorName(value: string): boolean {
  return /^[\p{L}\p{N}][\p{L}\p{N} ._'’-]{1,47}$/u.test(value.trim());
}

export function formatOperatorMessage(operators: readonly OperatorIdentity[], author: OperatorIdentity, text: string): string {
  const names = operators.map(operator => `${operator.name} (${operator.role})`).join(', ');
  return `[Two human operators are collaborating in this chat: ${names}. Treat each @(Name): prefix as message attribution.]
@(${author.name}): ${text.trim()}`;
}

export function splitOperatorMessage(text: string): { name: string | null; text: string } {
  const match = text.match(operatorPrefix);
  return match ? { name: match[1], text: text.slice(match[0].length) } : { name: null, text };
}

export function sharedTaskIds(snapshot: Snapshot, share: Pick<ActiveOperatorShare, 'taskIds' | 'projectIds'>): Set<string> {
  const explicit = new Set(share.taskIds);
  const projects = new Set(share.projectIds);
  return new Set(snapshot.tasks.filter(task => explicit.has(task.id) || (task.projectId !== null && projects.has(task.projectId))).map(task => task.id));
}

function safeAgent(agent: Agent): Agent {
  // Launchers can contain owner-local paths/arguments. Never spread future
  // owner-only agent fields into the visitor projection.
  return {
    id: agent.id, name: agent.name, description: agent.description,
    instructions: '', cwd: '', hostId: '', avatar: null,
    provider: agent.provider, model: agent.model, color: agent.color,
    sandbox: agent.sandbox, expertise: agent.expertise,
    responsibilities: agent.responsibilities, skills: agent.skills,
    collaborationEnabled: agent.collaborationEnabled,
  };
}

function safeProject(project: Project): Project {
  return { ...project, description: project.description, workspaces: [] };
}

function safeTask(task: Task): Task {
  return {
    id: task.id, agentId: task.agentId, title: task.title,
    archived: task.archived, status: task.status, createdAt: task.createdAt,
    updatedAt: task.updatedAt, parentTaskId: task.parentTaskId,
    channelId: task.channelId, projectId: task.projectId,
    provider: task.provider, model: task.model, modelSettings: task.modelSettings,
    cwd: '', hostId: '', nativeSessionId: null, sandbox: 'read-only',
  };
}

function safeMessage(message: Message): Message {
  return { ...message, attachments: [] };
}

/**
 * The visitor receives only explicitly selected task/project data. Local paths,
 * agent instructions, attachment references, events, approvals, queues and terminal data
 * are intentionally absent from the remote view.
 */
export function sharedSnapshot(snapshot: Snapshot, share: Pick<ActiveOperatorShare, 'taskIds' | 'projectIds'>): Snapshot {
  const ids = sharedTaskIds(snapshot, share);
  const tasks = snapshot.tasks.filter(task => ids.has(task.id)).map(safeTask);
  const agentIds = new Set(tasks.map(task => task.agentId));
  const projectIds = new Set(tasks.flatMap(task => task.projectId ? [task.projectId] : []));
  return {
    // Explicit projection: new owner-only Snapshot fields must never become
    // visitor-visible just because they were added to the desktop protocol.
    settings: { ...snapshot.settings, userName: '' },
    hosts: [],
    agents: snapshot.agents.filter(agent => agentIds.has(agent.id)).map(safeAgent),
    tasks,
    messages: snapshot.messages
      .filter(message => ids.has(message.taskId))
      // Saved agent profile/instruction records are owner-only system context.
      // Keep peer-attributed system records, which are part of a shared chat.
      .filter(message => message.role !== 'system' || !!message.senderAgentId)
      .map(safeMessage),
    events: [],
    channels: [],
    projects: snapshot.projects.filter(project => projectIds.has(project.id)).map(safeProject),
    collaborations: [],
    queuedMessages: [],
    approvalRequests: [],
  };
}
