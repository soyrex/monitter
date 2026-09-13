import assert from 'node:assert/strict';
import { ControllerDispatcher, type ControllerClient } from '../src/lib/controller/index.ts';

import { controllerSnapshot } from '../src/lib/controller/dispatcher.ts';
import { sharedSnapshot } from '../src/lib/operator-sharing.ts';
import type { Snapshot, Task, ApprovalRequest } from '../src/lib/types.ts';

const task = '11111111-1111-4111-8111-111111111111';
const terminal = '22222222-2222-4222-8222-222222222222';
const request = (id: string, action: string, params: Record<string, unknown>) => JSON.stringify({ version: 1, id, action, params });
const context = { authenticated: true as const, subject: 'paired-device:test' };
let sends = 0, cancellations = 0, resumes = 0;
const client: ControllerClient = {
  getSnapshot: async () => ({ hosts: [], agents: [], tasks: [], messages: [], events: [], channels: [], projects: [], collaborations: [], queuedMessages: [], approvalRequests: [], settings: { accent: '#000', theme: 'dark', interfaceScale: 125, showToolActivity: true, showReasoningSummaries: true, sendWithEnter: false, sidebarView: 'standard' } }),
  sendMessage: async () => { sends += 1; return client.getSnapshot(); },
  cancelTask: async () => { cancellations += 1; return client.getSnapshot(); },
  resumeTask: async () => { resumes += 1; return client.getSnapshot(); },
  listTerminals: async () => [{ id: terminal, title: 'safe', hostId: task, cwd: '/tmp', status: 'running', exitCode: null }],
  readTerminal: async (_id, afterSeq) => ({ chunks: [], nextSeq: afterSeq, status: 'running', exitCode: null, truncated: false }),
};
const sourceSnapshot = await client.getSnapshot();
sourceSnapshot.events = Array.from({ length: 120 }, (_, i) => ({
  id: String(i), taskId: task, kind: 'log' as const, title: 'test', detail: 'x'.repeat(5000), createdAt: i,
}));
const projected = controllerSnapshot(sourceSnapshot);
assert.equal(projected.events.length, 100);
assert.equal(projected.events[0].createdAt, 119);
assert.ok(projected.events.every(event => event.detail.length < 4100));
assert.equal(sourceSnapshot.events.length, 120);
assert.equal(sourceSnapshot.events[0].createdAt, 0);
assert.equal(sourceSnapshot.events[119].detail.length, 5000);
assert.equal(projected.messages, sourceSnapshot.messages);

// Shared visitors may see selected conversations, never owner-only approval
// payloads (which can contain local paths or secrets), including on shared tasks.
const sharedTask: Task = { id: task, agentId: 'agent', title: 'Shared chat', nativeSessionId: 'private-native-session', archived: false, status: 'idle', createdAt: 0, updatedAt: 0, parentTaskId: null, channelId: null, projectId: 'shared-project', hostId: 'private-host', cwd: '/private/folder', provider: 'claude', model: '', sandbox: 'harness-configured' };
const approval = (taskId: string): ApprovalRequest => ({ id: `approval-${taskId}`, taskId, provider: 'claude', runId: 'private-run', tool: 'Write', summary: 'Owner decision', detail: 'owner-only-approval-payload', risk: 'high', status: 'pending', createdAt: 0, resolvedAt: null, decision: null });
const sharingSource: Snapshot & { futureOwnerOnly: string } = {
  ...sourceSnapshot,
  settings: { ...sourceSnapshot.settings, userName: 'Alex Private' },
  tasks: [sharedTask, { ...sharedTask, id: 'unshared-task', projectId: null }],
  messages: [
    { id: 'shared-message', taskId: task, role: 'user', text: 'Selected content', createdAt: 0 },
    { id: 'owner-profile', taskId: task, role: 'system', text: 'Private saved profile', createdAt: 0 },
    { id: 'peer-system', taskId: task, role: 'system', senderAgentId: 'agent', text: 'Shared peer context', createdAt: 0 },
    { id: 'hidden-message', taskId: 'unshared-task', role: 'user', text: 'Unshared content', createdAt: 0 },
  ],
  approvalRequests: [approval(task), approval('unshared-task')],
  approvalRules: [{ id: 'private-rule', agentId: 'agent', hostId: 'private-host', provider: 'claude', cwd: '/private/rule-folder', tool: 'Write', summary: 'Owner rule', detail: 'owner-only-rule-payload', createdAt: 0, lastUsedAt: null, useCount: 0 }],
  futureOwnerOnly: 'future-private-field',
};
sharingSource.tasks[0].acp = { command: '/private/acp-command', args: ['private-acp-argument'] };
sharingSource.agents = [{ id: 'agent', name: 'ACP agent', description: '', instructions: 'private instructions', avatar: null, provider: 'acp', model: '', hostId: 'private-host', cwd: '/private/folder', color: '#000', sandbox: 'harness-configured', expertise: [], responsibilities: [], skills: [], collaborationEnabled: true, acp: { command: '/private/acp-agent', args: ['private-acp-agent-argument'] } }];
for (const selection of [{ taskIds: [task], projectIds: [] }, { taskIds: [], projectIds: ['shared-project'] }]) {
  const visitor = sharedSnapshot(sharingSource, selection);
  assert.deepEqual(visitor.tasks.map(item => item.id), [task]);
  assert.deepEqual(visitor.messages.map(item => item.id), ['shared-message', 'peer-system']);
  assert.equal(visitor.settings.userName, '');
  assert.deepEqual(visitor.approvalRequests, []);
  assert.deepEqual(visitor.approvalRules ?? [], []);
  assert.equal(visitor.tasks[0].cwd, '');
  assert.equal(visitor.tasks[0].nativeSessionId, null);
  assert.equal('futureOwnerOnly' in visitor, false);
  assert.ok(!JSON.stringify(visitor).includes('owner-only-approval-payload'));
  assert.ok(!JSON.stringify(visitor).includes('owner-only-rule-payload'));
  assert.ok(!JSON.stringify(visitor).includes('private-rule'));
  assert.ok(!JSON.stringify(visitor).includes('Alex Private'));
  assert.ok(!JSON.stringify(visitor).includes('Private saved profile'));
  assert.ok(!JSON.stringify(visitor).includes('private-acp'));
  assert.equal('acp' in visitor.tasks[0], false);
  assert.equal('acp' in visitor.agents[0], false);
}
assert.equal(sharingSource.approvalRequests.length, 2);
assert.equal(sharingSource.approvalRules?.length, 1);
assert.equal(sharingSource.tasks[0].cwd, '/private/folder');
const dispatcher = new ControllerDispatcher(client, { maxMutationReceipts: 3 });

// A current bridge can acknowledge a durable send without returning a large
// Snapshot. The dispatcher exposes that shape only when the peer negotiated it;
// legacy mobile clients continue to receive the snapshot-shaped response.
const receiptClient: ControllerClient = { ...client, sendMessage: async () => ({ accepted: true }) };
const receiptRequest = request('12121212-1212-4212-8212-121212121212', 'sendMessage', { taskId: task, text: 'receipt' });
const legacyReceipt = JSON.parse(await new ControllerDispatcher(receiptClient).dispatchJson(receiptRequest, context));
assert.equal(legacyReceipt.ok, true); assert.ok('tasks' in legacyReceipt.result);
const negotiatedReceipt = JSON.parse(await new ControllerDispatcher(receiptClient).dispatchJson(receiptRequest, context, { allowSendReceipt: true }));
assert.deepEqual(negotiatedReceipt.result, { accepted: true });

// This is the loopback transport proof: both directions are JSON strings and
// the authenticated context remains an in-process transport assertion.
let raw = await dispatcher.dispatchJson(request('33333333-3333-4333-8333-333333333333', 'getSnapshot', {}), context);
assert.equal(JSON.parse(raw).ok, true);
raw = await dispatcher.dispatchJson(request('44444444-4444-4444-8444-444444444444', 'listTerminals', {}), context);
assert.equal(JSON.parse(raw).result[0].id, terminal);
raw = await dispatcher.dispatchJson(request('55555555-5555-4555-8555-555555555555', 'readTerminal', { id: terminal, afterSeq: 0 }), context);
assert.equal(JSON.parse(raw).result.nextSeq, 0);

const sendId = '66666666-6666-4666-8666-666666666666';
await Promise.all([dispatcher.dispatchJson(request(sendId, 'sendMessage', { taskId: task, text: 'hello' }), context), dispatcher.dispatchJson(request(sendId, 'sendMessage', { taskId: task, text: 'hello' }), context)]);
assert.equal(sends, 1);
raw = await dispatcher.dispatchJson(request(sendId, 'sendMessage', { taskId: task, text: 'different' }), context);
assert.equal(JSON.parse(raw).error.code, 'id_conflict');

await dispatcher.dispatchJson(request('77777777-7777-4777-8777-777777777777', 'cancelTask', { taskId: task }), context);
await dispatcher.dispatchJson(request('88888888-8888-4888-8888-888888888888', 'resumeTask', { taskId: task }), context);
assert.equal(cancellations, 1); assert.equal(resumes, 1);
raw = await dispatcher.dispatchJson(request('99999999-9999-4999-8999-999999999999', 'cancelTask', { taskId: task }), context);
assert.equal(JSON.parse(raw).error.code, 'dedup_saturated');
raw = await dispatcher.dispatchJson(request('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', 'sendMessage', { taskId: task, text: 'x', arbitrary: true }), context);
assert.equal(JSON.parse(raw).error.code, 'invalid_request');
raw = await dispatcher.dispatchJson(request('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', 'sendMessage', { taskId: task, text: 'x' }), { authenticated: false, subject: 'spoofed' });
assert.equal(JSON.parse(raw).error.code, 'unauthenticated');

let releasePending: (() => void) | undefined;
const pending = new Promise<void>(resolve => { releasePending = resolve; });
const pendingClient: ControllerClient = { ...client, sendMessage: async () => { await pending; return client.getSnapshot(); } };
const saturated = new ControllerDispatcher(pendingClient, { maxMutationReceipts: 1 });
const pendingRequest = request('cccccccc-cccc-4ccc-8ccc-cccccccccccc', 'sendMessage', { taskId: task, text: 'pending' });
const inFlight = saturated.dispatchJson(pendingRequest, context);
await new Promise(resolve => setTimeout(resolve, 0));
raw = await saturated.dispatchJson(request('dddddddd-dddd-4ddd-8ddd-dddddddddddd', 'cancelTask', { taskId: task }), context);
assert.equal(JSON.parse(raw).error.code, 'dedup_saturated');
releasePending?.();
assert.equal(JSON.parse(await inFlight).ok, true);
console.log('Controller protocol loopback assertions passed.');
