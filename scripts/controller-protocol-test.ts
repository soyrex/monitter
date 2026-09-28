import assert from 'node:assert/strict';
import { ControllerDispatcher, type ControllerClient } from '../src/lib/controller/index.ts';

import { controllerSnapshot } from '../src/lib/controller/dispatcher.ts';
import { sharedSnapshot } from '../src/lib/operator-sharing.ts';
import { CONTROLLER_MAX_RESPONSE_BYTES } from '../src/lib/controller/protocol.ts';
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
assert.deepEqual(projected.messages, sourceSnapshot.messages);
assert.notEqual(projected.messages, sourceSnapshot.messages);

// A paired phone has no history-paging RPC in protocol v1. Long transcripts
// must therefore be projected to recent, visibly marked tails that still fit
// the encrypted controller response ceiling.
const hugeHistory: Snapshot = {
  ...sourceSnapshot,
  tasks: Array.from({ length: 8 }, (_, taskIndex) => ({
    id: `task-${taskIndex}`, agentId: `agent-${taskIndex}`, title: `Task ${taskIndex}`,
    nativeSessionId: null, archived: false, status: 'idle' as const, createdAt: 0, updatedAt: 0,
    parentTaskId: null, channelId: null, projectId: null, hostId: 'host', cwd: '/workspace', provider: 'codex' as const, model: 'model', sandbox: 'workspace-write' as const,
  })),
  agents: Array.from({ length: 8 }, (_, agentIndex) => ({
    id: `agent-${agentIndex}`, name: `Agent ${agentIndex}`, description: '', instructions: '', avatar: null,
    provider: 'codex' as const, model: 'model', hostId: 'host', cwd: '/workspace', color: '#000', sandbox: 'workspace-write' as const,
    expertise: [], responsibilities: [], skills: [], collaborationEnabled: true,
  })),
  messages: Array.from({ length: 1_600 }, (_, index) => ({
    id: `huge-message-${index}`, taskId: `task-${index % 8}`, role: 'assistant' as const,
    text: `${index}:${'x'.repeat(80_000)}`, createdAt: index,
  })),
  channels: [{ id: 'huge-channel', name: 'Huge channel', description: '', agentIds: ['agent-0'], messages: Array.from({ length: 100 }, (_, index) => ({
    id: `huge-channel-message-${index}`, role: 'assistant' as const, agentId: 'agent-0', taskId: null, text: 'y'.repeat(80_000), createdAt: index,
  })) }],
};
const hugeProjected = controllerSnapshot(hugeHistory);
assert.deepEqual(hugeProjected.tasks.map(item => item.id), hugeHistory.tasks.map(item => item.id));
assert.deepEqual(hugeProjected.agents.map(item => item.id), hugeHistory.agents.map(item => item.id));
assert.ok(hugeProjected.messages.length < hugeHistory.messages.length);
const hugeNotice = hugeProjected.messages.find(message => message.text === '[Earlier messages are available on the desktop.]');
assert.ok(hugeNotice);
assert.match(hugeNotice.id, /^[0-9a-f]{8}-[0-9a-f]{4}-5[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i);
const firstRetainedForNotice = hugeProjected.messages.find(message => message.taskId === hugeNotice.taskId && message.id !== hugeNotice.id);
assert.ok(firstRetainedForNotice && hugeProjected.messages.indexOf(hugeNotice) < hugeProjected.messages.indexOf(firstRetainedForNotice));
assert.ok(new TextEncoder().encode(JSON.stringify({ version: 1, id: '00000000-0000-4000-8000-000000000000', ok: true, result: hugeProjected })).byteLength <= CONTROLLER_MAX_RESPONSE_BYTES);

// The global content tail must not silently discard per-task truncation notices
// when a workspace has more tasks than its content-message allowance.
const manyTaskMessages = Array.from({ length: 650 }, (_, taskIndex) => ({
  id: `many-${taskIndex}`, taskId: `${taskIndex.toString(16).padStart(8, '0')}-0000-4000-8000-000000000000`, role: 'assistant' as const,
  text: 'Message', createdAt: taskIndex,
}));
const manyTaskProjected = controllerSnapshot({ ...sourceSnapshot, messages: manyTaskMessages });
assert.equal(manyTaskProjected.messages.filter(message => message.text === '[Earlier messages are available on the desktop.]').length, 50);

// Byte-budget trimming has the same omission guarantee as the initial global
// tail: every task retains content or receives its deterministic notice.
const byteHeavyMessages = Array.from({ length: 600 }, (_, taskIndex) => ({
  id: `byte-heavy-${taskIndex}`, taskId: `${taskIndex.toString(16).padStart(8, '0')}-0001-4000-8000-000000000000`, role: 'assistant' as const,
  text: 'z'.repeat(80_000), createdAt: taskIndex,
}));
const byteHeavyProjected = controllerSnapshot({ ...sourceSnapshot, messages: byteHeavyMessages });
for (const message of byteHeavyMessages) {
  assert.ok(byteHeavyProjected.messages.some(item => item.taskId === message.taskId), `Task ${message.taskId} must retain content or a history notice.`);
}
assert.ok(byteHeavyProjected.messages.some(message => message.text === '[Earlier messages are available on the desktop.]'));
assert.ok(new TextEncoder().encode(JSON.stringify({ version: 1, id: '00000000-0000-4000-8000-000000000000', ok: true, result: byteHeavyProjected })).byteLength <= CONTROLLER_MAX_RESPONSE_BYTES);

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

// Internal agents must never appear in any visitor projection, even when a crafted share
// carries their IDs alongside shared chat IDs.
const internalAgentSharing: typeof sharingSource = {
  ...sharingSource,
  agents: [...sharingSource.agents, { id: 'monitter-admin', name: 'Monitter Admin', description: '', instructions: '', avatar: null, provider: 'codex', model: '', hostId: 'private-host', cwd: '/private', color: '#000', sandbox: 'harness-configured', expertise: [], responsibilities: [], skills: [], collaborationEnabled: false, internal: true }],
  tasks: [...sharingSource.tasks, { ...sharedTask, id: 'admin-task', agentId: 'monitter-admin' }],
};
for (const selection of [{ taskIds: [task, 'admin-task'], projectIds: [] }, { taskIds: [], projectIds: ['shared-project'] }]) {
  const guarded = sharedSnapshot(internalAgentSharing, selection);
  assert.deepEqual(guarded.tasks.map(item => item.id), [task], 'Internal-agent tasks must never appear in a visitor projection.');
  assert.deepEqual(guarded.agents.map(item => item.id), ['agent'], 'Internal agents must never appear in a visitor projection.');
  assert.ok(!JSON.stringify(guarded).includes('Monitter Admin'));
  assert.ok(!JSON.stringify(guarded).includes('admin-task'));
}
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

// Mobile protocol v1 cannot request older pages. Its reads and legacy send
// acknowledgements use the same bounded compatibility projection.
const fullHistory: Snapshot = { ...sourceSnapshot, messages: Array.from({ length: 105 }, (_, i) => ({
  id: `legacy-message-${i}`, taskId: task, role: 'assistant', text: `History ${i}`, createdAt: i,
})) };
let legacyReads = 0, legacySends = 0;
const pagedClient: ControllerClient = {
  ...client,
  getSnapshot: async () => ({ ...fullHistory, messages: fullHistory.messages.slice(-64) }),
  getLegacySnapshot: async () => { legacyReads += 1; return fullHistory; },
  sendMessage: async () => { legacySends += 1; return { accepted: true }; },
};
const legacyDispatcher = new ControllerDispatcher(pagedClient);
const fullRead = JSON.parse(await legacyDispatcher.dispatchJson(request('10101010-1010-4010-8010-101010101010', 'getSnapshot', {}), context));
assert.equal(fullRead.ok, true);
assert.equal(fullRead.result.messages.length, 33);
assert.equal(fullRead.result.messages[0].text, '[Earlier messages are available on the desktop.]');
const legacySendRequest = request('20202020-2020-4020-8020-202020202020', 'sendMessage', { taskId: task, text: 'Legacy send' });
const fullReceipt = JSON.parse(await legacyDispatcher.dispatchJson(legacySendRequest, context));
assert.equal(fullReceipt.result.messages.length, 33);
assert.equal(JSON.parse(await legacyDispatcher.dispatchJson(legacySendRequest, context)).result.messages.length, 33);
assert.equal(legacySends, 1, 'Compatibility reads must not replay a mutation.');
assert.equal(legacyReads, 2);

// Large files never need a single relay frame: the controller accepts bounded
// chunks and only hands a complete, declared-size file to the scoped bridge.
let uploaded: { taskId: string; dataBase64: string; preview?: string } | null = null;
const uploadClient: ControllerClient = { ...client, storeAttachment: async (taskId, file, preview) => {
  uploaded = { taskId, dataBase64: file.dataBase64, ...(preview ? { preview } : {}) };
  return { id: 'abababab-abab-4bab-8bab-abababababab', name: file.filename, mimeType: file.mimeType, size: atob(file.dataBase64).length, path: '' };
} };
const uploadDispatcher = new ControllerDispatcher(uploadClient);
const uploadOptions = { allowUploads: true };
const preview = 'data:image/png;base64,aGVsbG8=';
const begin = JSON.parse(await uploadDispatcher.dispatchJson(request('13131313-1313-4313-8313-131313131313', 'beginAttachmentUpload', { taskId: task, filename: 'hello.txt', mimeType: 'text/plain', size: 5, previewDataUrl: preview }), context, uploadOptions));
assert.equal(begin.ok, true); const uploadId = begin.result.uploadId;
assert.equal(JSON.parse(await uploadDispatcher.dispatchJson(request('14141414-1414-4414-8414-141414141414', 'appendAttachmentUpload', { uploadId, dataBase64: 'aGVsbG8=' }), context, uploadOptions)).ok, true);
const finished = JSON.parse(await uploadDispatcher.dispatchJson(request('15151515-1515-4515-8515-151515151515', 'finishAttachmentUpload', { uploadId }), context, uploadOptions));
assert.equal(finished.result.name, 'hello.txt'); assert.deepEqual(uploaded, { taskId: task, dataBase64: 'aGVsbG8=', preview });
const traversal = JSON.parse(await uploadDispatcher.dispatchJson(request('16161616-1616-4616-8616-161616161616', 'beginAttachmentUpload', { taskId: task, filename: '../secret', mimeType: 'text/plain', size: 1 }), context, uploadOptions));
assert.equal(traversal.error.code, 'invalid_request');
const incomplete = JSON.parse(await uploadDispatcher.dispatchJson(request('17171717-1717-4717-8717-171717171717', 'beginAttachmentUpload', { taskId: task, filename: 'short.txt', mimeType: 'text/plain', size: 2 }), context, uploadOptions));
assert.equal(JSON.parse(await uploadDispatcher.dispatchJson(request('18181818-1818-4818-8818-181818181818', 'finishAttachmentUpload', { uploadId: incomplete.result.uploadId }), context, uploadOptions)).error.code, 'bridge_error');

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
