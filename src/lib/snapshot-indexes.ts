import type { Agent, ApprovalRequest, Channel, Host, Message, Project, RunEvent, Snapshot, Task } from '$lib/types';

/**
 * A launch snapshot is immutable from the UI's point of view. Build the common
 * lookup tables once in the owning surface, then pass them to split panes so a
 * four-pane workspace does not repeatedly scan the same transcript.
 */
export type SnapshotIndexes = {
  snapshot: Snapshot;
  agentById: ReadonlyMap<string, Agent>;
  hostById: ReadonlyMap<string, Host>;
  projectById: ReadonlyMap<string, Project>;
  taskById: ReadonlyMap<string, Task>;
  /** Unarchived tasks, plus the activity sort shared by every split pane. */
  activeTasks: Task[];
  activityTasks: Task[];
  activeTasksByAgent: ReadonlyMap<string, Task[]>;
  activeTasksByProject: ReadonlyMap<string, Task[]>;
  activityTasksByAgent: ReadonlyMap<string, Task[]>;
  activityTasksByProject: ReadonlyMap<string, Task[]>;
  /**
   * User-visible agents (excluding internal agents and their tasks). The raw
   * maps above remain authoritative for backend-linked lookups; these arrays
   * and the visibility-keyed maps are the only sources the UI must use when
   * enumerating agents/tasks the user can see and select.
   */
  visibleAgents: Agent[];
  visibleAgentIds: ReadonlySet<string>;
  visibleTasks: Task[];
  visibleActiveTasks: Task[];
  visibleActivityTasks: Task[];
  visibleActiveTasksByAgent: ReadonlyMap<string, Task[]>;
  visibleActiveTasksByProject: ReadonlyMap<string, Task[]>;
  visibleActivityTasksByAgent: ReadonlyMap<string, Task[]>;
  visibleActivityTasksByProject: ReadonlyMap<string, Task[]>;
  localHost: Host | null;
  defaultAgent: Agent | null;
  messagesByTask: ReadonlyMap<string, Message[]>;
  eventsByTask: ReadonlyMap<string, RunEvent[]>;
  approvalsByTask: ReadonlyMap<string, ApprovalRequest[]>;
  queuedByTask: ReadonlyMap<string, Snapshot['queuedMessages']>;
  queuedByChannel: ReadonlyMap<string, Snapshot['queuedMessages']>;
  tasksByParent: ReadonlyMap<string, Task[]>;
  runningTasksByChannel: ReadonlyMap<string, Task[]>;
  channelById: ReadonlyMap<string, Channel>;
};

function append<T>(items: Map<string, T[]>, key: string | null | undefined, value: T) {
  if (!key) return;
  const values = items.get(key);
  if (values) values.push(value);
  else items.set(key, [value]);
}

function readonlyValues<T>(items: Map<string, T[]>): ReadonlyMap<string, T[]> {
  return items;
}

export function createSnapshotIndexes(snapshot: Snapshot): SnapshotIndexes {
  const agentById = new Map(snapshot.agents.map(agent => [agent.id, agent]));
  const hostById = new Map(snapshot.hosts.map(host => [host.id, host]));
  const projectById = new Map(snapshot.projects.map(project => [project.id, project]));
  const taskById = new Map(snapshot.tasks.map(task => [task.id, task]));
  const channelById = new Map(snapshot.channels.map(channel => [channel.id, channel]));
  const messagesByTask = new Map<string, Message[]>();
  const eventsByTask = new Map<string, RunEvent[]>();
  const approvalsByTask = new Map<string, ApprovalRequest[]>();
  const queuedByTask = new Map<string, Snapshot['queuedMessages']>();
  const queuedByChannel = new Map<string, Snapshot['queuedMessages']>();
  const tasksByParent = new Map<string, Task[]>();
  const runningTasksByChannel = new Map<string, Task[]>();
  const activeTasksByAgent = new Map<string, Task[]>();
  const activeTasksByProject = new Map<string, Task[]>();
  const activityTasksByAgent = new Map<string, Task[]>();
  const activityTasksByProject = new Map<string, Task[]>();
  const activeTasks = snapshot.tasks.filter(task => !task.archived);
  const activityTasks = activeTasks.filter(task => !task.channelId).toSorted((left, right) =>
    Number(right.status === 'running') - Number(left.status === 'running') || right.updatedAt - left.updatedAt || left.id.localeCompare(right.id));
  // The internal flag hides a bootstrap agent (and its tasks) from every user-facing surface.
  // Backend is authoritative: an agent without the flag is user-visible, a flagged agent is internal.
  const visibleAgents = snapshot.agents.filter(agent => !agent.internal);
  const visibleAgentIds = new Set(visibleAgents.map(agent => agent.id));
  const visibleTasks = snapshot.tasks.filter(task => visibleAgentIds.has(task.agentId));
  const visibleActiveTasks = visibleTasks.filter(task => !task.archived);
  const visibleActivityTasks = visibleActiveTasks.filter(task => !task.channelId).toSorted((left, right) =>
    Number(right.status === 'running') - Number(left.status === 'running') || right.updatedAt - left.updatedAt || left.id.localeCompare(right.id));
  const visibleActiveTasksByAgent = new Map<string, Task[]>();
  const visibleActiveTasksByProject = new Map<string, Task[]>();
  const visibleActivityTasksByAgent = new Map<string, Task[]>();
  const visibleActivityTasksByProject = new Map<string, Task[]>();

  for (const task of snapshot.tasks) {
    append(tasksByParent, task.parentTaskId, task);
    if (task.status === 'running') append(runningTasksByChannel, task.channelId, task);
  }
  for (const task of activeTasks) {
    append(activeTasksByAgent, task.agentId, task);
    append(activeTasksByProject, task.projectId || 'unassigned', task);
  }
  for (const task of activityTasks) {
    append(activityTasksByAgent, task.agentId, task);
    append(activityTasksByProject, task.projectId || 'unassigned', task);
  }
  for (const task of visibleActiveTasks) {
    append(visibleActiveTasksByAgent, task.agentId, task);
    append(visibleActiveTasksByProject, task.projectId || 'unassigned', task);
  }
  for (const task of visibleActivityTasks) {
    append(visibleActivityTasksByAgent, task.agentId, task);
    append(visibleActivityTasksByProject, task.projectId || 'unassigned', task);
  }
  for (const message of snapshot.messages) append(messagesByTask, message.taskId, message);
  for (const event of snapshot.events) append(eventsByTask, event.taskId, event);
  for (const request of snapshot.approvalRequests) append(approvalsByTask, request.taskId, request);
  for (const queued of snapshot.queuedMessages) {
    append(queuedByTask, queued.taskId, queued);
    append(queuedByChannel, queued.channelId, queued);
  }
  for (const events of eventsByTask.values()) events.sort((left, right) => left.createdAt - right.createdAt);

  return {
    snapshot,
    agentById,
    hostById,
    projectById,
    taskById,
    activeTasks,
    activityTasks,
    activeTasksByAgent: readonlyValues(activeTasksByAgent),
    activeTasksByProject: readonlyValues(activeTasksByProject),
    activityTasksByAgent: readonlyValues(activityTasksByAgent),
    activityTasksByProject: readonlyValues(activityTasksByProject),
    visibleAgents,
    visibleAgentIds,
    visibleTasks,
    visibleActiveTasks,
    visibleActivityTasks,
    visibleActiveTasksByAgent: readonlyValues(visibleActiveTasksByAgent),
    visibleActiveTasksByProject: readonlyValues(visibleActiveTasksByProject),
    visibleActivityTasksByAgent: readonlyValues(visibleActivityTasksByAgent),
    visibleActivityTasksByProject: readonlyValues(visibleActivityTasksByProject),
    localHost: snapshot.hosts.find(host => host.kind === 'local') ?? null,
    defaultAgent: visibleAgents.find(agent => agent.provider === 'codex') ?? visibleAgents[0] ?? null,
    channelById,
    messagesByTask: readonlyValues(messagesByTask),
    eventsByTask: readonlyValues(eventsByTask),
    approvalsByTask: readonlyValues(approvalsByTask),
    queuedByTask: readonlyValues(queuedByTask),
    queuedByChannel: readonlyValues(queuedByChannel),
    tasksByParent: readonlyValues(tasksByParent),
    runningTasksByChannel: readonlyValues(runningTasksByChannel),
  };
}

/** Returns an already-grouped active task set without scanning the snapshot. Internal agents are excluded. */
export function activeTasksForWorkspace(indexes: SnapshotIndexes | null | undefined, scope: string): Task[] {
  if (!indexes) return [];
  if (scope === 'all') return indexes.visibleActiveTasks;
  if (scope.startsWith('agent:')) return indexes.visibleActiveTasksByAgent.get(scope.slice(6)) ?? [];
  if (scope.startsWith('project:')) return indexes.visibleActiveTasksByProject.get(scope.slice(8)) ?? [];
  return [];
}

export function activityTasksForWorkspace(indexes: SnapshotIndexes | null | undefined, scope: string): Task[] {
  if (!indexes) return [];
  if (scope === 'all') return indexes.visibleActivityTasks;
  if (scope.startsWith('agent:')) return indexes.visibleActivityTasksByAgent.get(scope.slice(6)) ?? [];
  if (scope.startsWith('project:')) return indexes.visibleActivityTasksByProject.get(scope.slice(8)) ?? [];
  return [];
}
