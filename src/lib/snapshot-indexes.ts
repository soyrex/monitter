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
    localHost: snapshot.hosts.find(host => host.kind === 'local') ?? null,
    defaultAgent: snapshot.agents.find(agent => agent.provider === 'codex') ?? snapshot.agents[0] ?? null,
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

/** Returns an already-grouped active task set without scanning the snapshot. */
export function activeTasksForWorkspace(indexes: SnapshotIndexes | null | undefined, scope: string): Task[] {
  if (!indexes) return [];
  if (scope === 'all') return indexes.activeTasks;
  if (scope.startsWith('agent:')) return indexes.activeTasksByAgent.get(scope.slice(6)) ?? [];
  if (scope.startsWith('project:')) return indexes.activeTasksByProject.get(scope.slice(8)) ?? [];
  return [];
}

export function activityTasksForWorkspace(indexes: SnapshotIndexes | null | undefined, scope: string): Task[] {
  if (!indexes) return [];
  if (scope === 'all') return indexes.activityTasks;
  if (scope.startsWith('agent:')) return indexes.activityTasksByAgent.get(scope.slice(6)) ?? [];
  if (scope.startsWith('project:')) return indexes.activityTasksByProject.get(scope.slice(8)) ?? [];
  return [];
}
