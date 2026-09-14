import type { ApprovalRequest, Channel, Message, RunEvent, Snapshot, Task } from '$lib/types';

/**
 * A launch snapshot is immutable from the UI's point of view. Build the common
 * lookup tables once in the owning surface, then pass them to split panes so a
 * four-pane workspace does not repeatedly scan the same transcript.
 */
export type SnapshotIndexes = {
  snapshot: Snapshot;
  taskById: ReadonlyMap<string, Task>;
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
  const taskById = new Map(snapshot.tasks.map(task => [task.id, task]));
  const channelById = new Map(snapshot.channels.map(channel => [channel.id, channel]));
  const messagesByTask = new Map<string, Message[]>();
  const eventsByTask = new Map<string, RunEvent[]>();
  const approvalsByTask = new Map<string, ApprovalRequest[]>();
  const queuedByTask = new Map<string, Snapshot['queuedMessages']>();
  const queuedByChannel = new Map<string, Snapshot['queuedMessages']>();
  const tasksByParent = new Map<string, Task[]>();
  const runningTasksByChannel = new Map<string, Task[]>();

  for (const task of snapshot.tasks) {
    append(tasksByParent, task.parentTaskId, task);
    if (task.status === 'running') append(runningTasksByChannel, task.channelId, task);
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
    taskById,
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
