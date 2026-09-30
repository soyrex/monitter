import assert from 'node:assert/strict';
import { applyUiDelta, mergeChannelMessagesById, mergeMessagesById } from '../src/lib/ui-sync.ts';

const messages = [
  { id: 'm1', taskId: 'task', role: 'user', text: 'first', createdAt: 1 },
  { id: 'm2', taskId: 'task', role: 'assistant', text: 'second', createdAt: 2 },
];
const channelMessages = [
  { id: 'c1', role: 'user', agentId: null, taskId: 'task', text: 'first', createdAt: 1 },
  { id: 'c2', role: 'assistant', agentId: null, taskId: 'task', text: 'second', createdAt: 2 },
];

assert.strictEqual(mergeMessagesById(messages, []), messages, 'empty task updates must preserve the loaded transcript array');
assert.strictEqual(mergeMessagesById(messages, [], []), messages, 'explicit empty removals must remain a no-op');
assert.strictEqual(mergeChannelMessagesById(channelMessages, []), channelMessages, 'empty channel updates must preserve the loaded history array');

const snapshot = {
  hosts: [], agents: [], tasks: [], messages, events: [], channels: [
    { id: 'channel', name: 'before', description: '', agentIds: [], messages: channelMessages },
  ], projects: [], collaborations: [], queuedMessages: [], approvalRequests: [], settings: {},
};
const applied = applyUiDelta(snapshot, 'r1', {
  revision: 'r2', snapshot: null,
  delta: {
    fromRevision: 'r1',
    metadata: { ...snapshot, messages: [], channels: [{ ...snapshot.channels[0], name: 'after', messages: [] }] },
    messages: [], removedMessageIds: [], retainedChannelIds: ['channel'], retainedSubagentTranscriptIds: [],
  },
});
assert.equal(applied.kind, 'delta');
assert.strictEqual(applied.snapshot.messages, messages, 'metadata-only delta must retain paged task history');
assert.equal(applied.snapshot.channels[0].name, 'after', 'metadata must still advance');
assert.strictEqual(applied.snapshot.channels[0].messages, channelMessages, 'retained channel history must stay shared');

const merged = mergeMessagesById(messages, [{ ...messages[1], text: 'updated' }], ['m1']);
assert.deepEqual(merged, [{ ...messages[1], text: 'updated' }], 'non-empty updates and removals must still apply');
console.log('UI sync cache no-op sharing and changed-message merge passed.');
