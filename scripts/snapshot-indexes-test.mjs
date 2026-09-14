import assert from 'node:assert/strict';
import { createSnapshotIndexes } from '../src/lib/snapshot-indexes.ts';

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
console.log('snapshot index behaviors passed');
