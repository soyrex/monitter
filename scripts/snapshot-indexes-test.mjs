import assert from 'node:assert/strict';
import { createSnapshotIndexes, activeTasksForWorkspace, activityTasksForWorkspace } from '../src/lib/snapshot-indexes.ts';

const task = id => ({ id, agentId: 'agent', title: id, nativeSessionId: null, archived: false, status: 'running', createdAt: 1, updatedAt: 1, parentTaskId: null, channelId: 'channel', projectId: null, hostId: 'host', cwd: '/tmp', provider: 'codex', model: 'test', sandbox: 'read-only' });
const snapshot = {
  hosts: [], agents: [], tasks: [task('root'), { ...task('child'), parentTaskId: 'root', channelId: null, status: 'completed' }],
  messages: [
    { id: 'm2', taskId: 'root', role: 'assistant', text: 'later', createdAt: 2 },
    { id: 'm1', taskId: 'root', role: 'user', text: 'earlier', createdAt: 1 },
  ],
  events: [
    { id: 'e2', taskId: 'root', kind: 'output', title: 'later', detail: '', createdAt: 2 },
    { id: 'e1', taskId: 'root', kind: 'status', title: 'earlier', detail: '', createdAt: 1 },
  ],
  channels: [{ id: 'channel', name: 'Channel', description: '', agentIds: [], messages: [] }],
  projects: [], settings: {}, collaborations: [], queuedMessages: [
    { id: 'q1', taskId: 'root', channelId: 'channel', text: 'queued', attachmentIds: [], createdAt: 1, status: 'queued' },
  ], approvalRequests: [],
};

const indexes = createSnapshotIndexes(snapshot);
assert.equal(indexes.taskById.get('root'), snapshot.tasks[0]);
assert.deepEqual(indexes.messagesByTask.get('root').map(item => item.id), ['m2', 'm1'], 'message order must remain snapshot order');
assert.deepEqual(indexes.eventsByTask.get('root').map(item => item.id), ['e1', 'e2'], 'events must be normalized once');
assert.deepEqual(indexes.tasksByParent.get('root').map(item => item.id), ['child']);
assert.deepEqual(indexes.runningTasksByChannel.get('channel').map(item => item.id), ['root']);
assert.deepEqual(indexes.queuedByTask.get('root').map(item => item.id), ['q1']);
assert.deepEqual(indexes.queuedByChannel.get('channel').map(item => item.id), ['q1']);
assert.equal(indexes.channelById.get('channel'), snapshot.channels[0]);
assert.equal(indexes.messagesByTask.get('missing'), undefined);
// Internal-agent visibility boundary: empty agents means every task belongs to a hidden agent,
// so all visible arrays must be empty while the raw maps remain populated for backend lookups.
assert.deepEqual(indexes.visibleAgents, []);
assert.deepEqual([...indexes.visibleAgentIds], []);
assert.deepEqual(indexes.visibleTasks, []);
assert.deepEqual(indexes.visibleActiveTasks, []);
assert.deepEqual(indexes.visibleActivityTasks, []);
assert.equal(indexes.taskById.get('root'), snapshot.tasks[0], 'raw taskById must remain authoritative');
assert.equal(indexes.defaultAgent, null);
assert.deepEqual(activeTasksForWorkspace(indexes, 'all'), []);
assert.deepEqual(activityTasksForWorkspace(indexes, 'all'), []);
assert.deepEqual(activeTasksForWorkspace(indexes, 'agent:agent'), []);
assert.deepEqual(activityTasksForWorkspace(indexes, 'project:unassigned'), []);

const visibleAgent = { id: 'visible', name: 'Visible', description: '', instructions: '', avatar: null, provider: 'codex', model: '', hostId: 'host', cwd: '/tmp', color: '#000', sandbox: 'read-only', expertise: [], responsibilities: [], skills: [], collaborationEnabled: true };
const internalAgent = { ...visibleAgent, id: 'internal-admin', name: 'Monitter Admin', internal: true };
const visibleSnapshot = {
  ...snapshot,
  agents: [visibleAgent, internalAgent],
  tasks: [
    { ...task('public'), agentId: 'visible', channelId: null },
    { ...task('admin'), agentId: 'internal-admin', channelId: null },
  ],
  approvalRequests: [
    { id: 'r1', taskId: 'public', provider: 'codex', runId: 'r1', tool: 'Write', summary: 's', detail: '', risk: 'low', status: 'pending', createdAt: 1, resolvedAt: null, decision: null },
    { id: 'r2', taskId: 'admin', provider: 'codex', runId: 'r2', tool: 'Write', summary: 's', detail: '', risk: 'low', status: 'pending', createdAt: 1, resolvedAt: null, decision: null },
  ],
};
const mixed = createSnapshotIndexes(visibleSnapshot);
assert.deepEqual(mixed.visibleAgents.map(item => item.id), ['visible']);
assert.equal(mixed.visibleAgentIds.has('internal-admin'), false);
assert.deepEqual(mixed.visibleTasks.map(item => item.id), ['public']);
assert.deepEqual(mixed.visibleActivityTasks.map(item => item.id), ['public']);
assert.equal(mixed.agentById.get('internal-admin'), internalAgent, 'raw map must retain internal agent');
assert.equal(mixed.taskById.get('admin'), visibleSnapshot.tasks[1], 'raw map must retain internal task');
assert.equal(mixed.defaultAgent?.id, 'visible');
assert.deepEqual(activeTasksForWorkspace(mixed, 'all').map(item => item.id), ['public']);
assert.deepEqual(activityTasksForWorkspace(mixed, 'all').map(item => item.id), ['public']);
assert.deepEqual(activeTasksForWorkspace(mixed, 'agent:internal-admin'), []);
console.log('snapshot index behaviors passed');
