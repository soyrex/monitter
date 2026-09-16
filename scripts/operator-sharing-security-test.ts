import assert from 'node:assert/strict';
import {
  createOperatorScopedBridge,
  formatOperatorMessage,
  sharedSnapshot,
  splitOperatorMessage,
  validOperatorName,
  type ActiveOperatorShare,
} from '../src/lib/operator-sharing.ts';
import type { ModelCatalog, Snapshot, Task } from '../src/lib/types.ts';

const selectedId = '11111111-1111-4111-8111-111111111111';
const hiddenId = '22222222-2222-4222-8222-222222222222';
const task = (id: string, overrides: Partial<Task> = {}): Task => ({
  id, agentId: 'agent', title: id === selectedId ? 'Selected' : 'Hidden',
  nativeSessionId: `native-${id}`, archived: false, status: 'idle', createdAt: 1,
  updatedAt: 2, parentTaskId: null, channelId: null, projectId: null,
  hostId: 'private-host', cwd: '/private/workspace', provider: 'codex', model: 'private-model',
  sandbox: 'workspace-write', ...overrides,
});
const settings = { accent: '#123456', theme: 'dark' as const, interfaceScale: 125,
  showToolActivity: true, showReasoningSummaries: true, sendWithEnter: false,
  sidebarView: 'standard' as const, userName: 'Private owner' };
const source: Snapshot & { futureOwnerSecret: string } = {
  hosts: [{ id: 'private-host', name: 'Mac', kind: 'local', address: '', user: '', port: 22,
    identityFile: '/private/key', defaultCwd: '/private', codexPath: '/private/codex',
    claudePath: '', opencodePath: '', hermesPath: '' }],
  agents: [{ id: 'agent', name: 'Agent', description: 'Public description', instructions: 'secret instructions',
    avatar: 'data:image/png;base64,c2VjcmV0', provider: 'codex', model: 'private-model', hostId: 'private-host',
    cwd: '/private/workspace', color: '#000', sandbox: 'workspace-write', expertise: ['private expertise'],
    responsibilities: ['private responsibility'], skills: ['private skill'], collaborationEnabled: true }],
  tasks: [task(selectedId), task(hiddenId), task('archived', { archived: true }), task('channel', { channelId: 'channel-1' })],
  messages: [
    { id: 'visible', taskId: selectedId, role: 'assistant', text: 'hello', createdAt: 1,
      phase: 'final_answer',
      attachments: [{ id: 'attachment', name: 'secret.txt', mimeType: 'text/plain', size: 10, path: '/private/secret.txt' }] },
    { id: 'profile', taskId: selectedId, role: 'system', text: 'secret system profile', createdAt: 1 },
    { id: 'hidden', taskId: hiddenId, role: 'user', text: 'hidden chat', createdAt: 1 },
  ],
  events: [{ id: 'event', taskId: selectedId, kind: 'log', title: 'secret log', detail: '/private/log', createdAt: 1 }],
  channels: [{ id: 'channel-1', name: 'secret channel', description: '', agentIds: [], messages: [] }],
  projects: [], settings, collaborations: [],
  queuedMessages: [{ id: 'queue', taskId: selectedId, channelId: null, text: 'secret queued text', attachmentIds: [], createdAt: 1, status: 'queued' }],
  approvalRequests: [{ id: 'approval', taskId: selectedId, provider: 'codex', runId: 'run', tool: 'shell',
    summary: 'secret approval', detail: '/private/approval', risk: 'high', status: 'pending', createdAt: 1,
    resolvedAt: null, decision: null }],
  approvalRules: [{ id: 'rule', agentId: 'agent', hostId: 'private-host', provider: 'codex', cwd: '/private',
    tool: 'shell', summary: 'secret rule', detail: 'secret', createdAt: 1, lastUsedAt: null, useCount: 0 }],
  futureOwnerSecret: 'future private value',
};

const grant = (): ActiveOperatorShare => ({
  primary: { name: 'Alex', role: 'primary user' }, visitor: { name: 'Sam', role: 'visitor' },
  taskIds: [selectedId], projectIds: [],
});

assert.equal(validOperatorName('Alex'), true);
assert.equal(validOperatorName('A'), false);
assert.equal(validOperatorName('Alex\nVisitor'), false);
const formatted = formatOperatorMessage([grant().primary, grant().visitor], grant().visitor, '  preserve this  ');
assert.match(formatted, /Alex \(primary user\).*Sam \(visitor\)/);
assert.ok(formatted.endsWith('@(Sam):   preserve this  '), 'Operator formatting must preserve raw user text.');
assert.deepEqual(splitOperatorMessage('@(Sam): hello'), { name: 'Sam', text: 'hello' });

const visitor = sharedSnapshot(source, grant());
assert.deepEqual(visitor.tasks.map(item => item.id), [selectedId]);
assert.deepEqual(visitor.messages.map(item => item.id), ['visible']);
assert.deepEqual(visitor.messages[0].attachments, [{ id: 'attachment', name: 'secret.txt', mimeType: 'text/plain', size: 10, path: '' }]);
assert.equal(visitor.messages[0].phase, 'final_answer');
assert.deepEqual(visitor.hosts, []);
assert.deepEqual(visitor.events, []);
assert.deepEqual(visitor.channels, []);
assert.deepEqual(visitor.queuedMessages, []);
assert.deepEqual(visitor.approvalRequests, []);
assert.deepEqual(visitor.approvalRules, []);
const serialized = JSON.stringify(visitor);
for (const secret of ['/private', 'secret instructions', 'secret system profile', 'hidden chat',
  'secret queued text', 'secret approval', 'future private value', 'native-']) assert.ok(!serialized.includes(secret), secret);
assert.equal(source.tasks[0].cwd, '/private/workspace', 'Projection must not mutate owner state.');
assert.equal(source.messages[0].attachments?.[0].path, '/private/secret.txt');

const composerSource: Snapshot = {
  ...source,
  tasks: [task(selectedId, { model: 'model-id', modelSettings: { model: 'model-id', reasoningEffort: null, fastMode: null } })],
  messages: [
    { id: 'context-message', taskId: selectedId, role: 'user', text: 'shared', createdAt: 5 },
    { id: 'other-context-clear', taskId: hiddenId, role: 'system', text: 'Context Cleared', createdAt: 99 },
  ],
};

// Even a crafted share carrying an internal agent's ID must never surface it.
const internalAgentSource: Snapshot = {
  ...source,
  agents: [...source.agents, { id: 'monitter-admin', name: 'Monitter Admin', description: '', instructions: '', avatar: null, provider: 'codex', model: '', hostId: 'private-host', cwd: '/private', color: '#000', sandbox: 'harness-configured', expertise: [], responsibilities: [], skills: [], collaborationEnabled: false, internal: true }],
  tasks: [...source.tasks, { ...task(selectedId, { id: 'admin-task' }), agentId: 'monitter-admin' }],
};
const craftedGrant = (): ActiveOperatorShare => ({
  primary: { name: 'Alex', role: 'primary user' }, visitor: { name: 'Sam', role: 'visitor' },
  taskIds: [selectedId, 'admin-task'], projectIds: [],
});
const craftedVisitor = sharedSnapshot(internalAgentSource, craftedGrant());
assert.deepEqual(craftedVisitor.tasks.map(item => item.id), [selectedId], 'Internal-agent tasks must never appear in a visitor projection.');
assert.deepEqual(craftedVisitor.agents.map(item => item.id), ['agent'], 'Internal agents must never appear in a visitor projection.');
assert.ok(!JSON.stringify(craftedVisitor).includes('Monitter Admin'));
assert.ok(!JSON.stringify(craftedVisitor).includes('admin-task'));

let active: ActiveOperatorShare | null = grant();
let sent = 0;
let sentText = '';
const bridge = createOperatorScopedBridge({
  getSnapshot: async () => source,
  sendMessage: async (_taskId, text) => { sent += 1; sentText = text; return source; },
}, () => active);
assert.equal('storeAttachment' in bridge, false, 'Older bridges without attachment storage must not advertise uploads.');
assert.equal((await bridge.getSnapshot()).sharing?.uploads, undefined, 'Older bridges must not advertise upload limits.');
await bridge.sendMessage(selectedId, 'hello');
assert.equal(sent, 1);
assert.ok(sentText.endsWith('@(Sam): hello'));
await assert.rejects(bridge.sendMessage(hiddenId, 'no'), /not shared/i);
await assert.rejects(bridge.sendMessage('archived', 'no'), /not shared/i);
await assert.rejects(bridge.sendMessage('channel', 'no'), /not shared/i);
assert.equal(sent, 1);
await assert.rejects(bridge.listTerminals(), /only reading and messaging/i);
await assert.rejects(bridge.readTerminal('terminal', 0), /only reading and messaging/i);
await assert.rejects(bridge.cancelTask(selectedId), /only reading and messaging/i);
await assert.rejects(bridge.resumeTask(selectedId), /only reading and messaging/i);

// Composer metadata is an owner-side allowlist. It carries exactly the normal
// per-task context display state, never the full usage overview or catalog diagnostics.
let catalogReads = 0;
const composerBridge = createOperatorScopedBridge({
  getSnapshot: async () => composerSource,
  sendMessage: async () => composerSource,
  getUsageOverview: async policy => {
    assert.equal(policy, 'cache-only');
    return {
      generatedAt: 9, capturedSince: 1,
      subscriptions: [{ provider: 'codex', hostId: 'private-host', source: '/private/account', state: 'available', planType: 'secret plan', fetchedAt: 1, staleAfter: 2, lastAttemptAt: 1, windows: [{ key: 'secret', label: 'Secret allowance', metric: 'tokens', usedPercent: 50, used: 50, limit: 100, unit: 'tokens', resetsAt: 3 }], balances: [], error: '/private/error' }],
      providerTotals: [],
      recentRuns: [{ runId: 'private-run', taskId: selectedId, provider: 'codex', configuredModel: 'model-id', startedAt: 6, finishedAt: 7, final: true, tokens: { input: 1, output: 2, cacheRead: 3, cacheWrite: 4, reasoning: 5, total: 6 }, costUsd: 7, durationMs: 8, apiDurationMs: 9, providerTurns: 10, context: { used: 123, size: 456 } }],
    };
  },
  getModelCatalog: async () => {
    catalogReads += 1;
    return { models: [{ id: 'model-id', name: 'Safe model label', description: '/private/description', reasoningEfforts: [{ id: 'medium', description: '/private/effort' }], defaultEffort: 'medium', supportsFast: true, fastDescription: '/private/fast' }], current: { model: 'model-id', reasoningEffort: 'high', fastMode: true }, source: '/private/catalog', warning: '/private/warning' };
  },
}, () => active);
const composerVisitor = await composerBridge.getSnapshot();
assert.deepEqual(composerVisitor.sharing?.composerByTask, {
  [selectedId]: { model: { id: 'model-id', label: 'Safe model label' }, reasoningEffort: 'high', fastMode: true, context: { status: 'available', used: 123, size: 456, usedPercent: (123 / 456) * 100, model: 'model-id' } },
});
await composerBridge.getSnapshot();
assert.equal(catalogReads, 1, 'Catalog lookup must be cached across sharing polling.');
const composerSerialized = JSON.stringify(composerVisitor);
for (const secret of ['/private/account', 'secret plan', 'Secret allowance', 'private-run', '/private/description', '/private/effort', '/private/fast', '/private/catalog', '/private/warning']) assert.ok(!composerSerialized.includes(secret), secret);

let finishSlowCatalog!: (catalog: ModelCatalog) => void;
const slowCatalog = new Promise<ModelCatalog>(resolve => { finishSlowCatalog = resolve; });
const delayedCatalogBridge = createOperatorScopedBridge({
  getSnapshot: async () => composerSource,
  sendMessage: async () => composerSource,
  getModelCatalog: async () => slowCatalog,
}, () => active);
const beforeCatalog = await delayedCatalogBridge.getSnapshot();
assert.equal(beforeCatalog.sharing?.composerByTask?.[selectedId]?.model.label, 'model-id', 'A slow catalog must not hold a shared snapshot open.');
finishSlowCatalog({ models: [{ id: 'model-id', name: 'Late safe label', description: '', reasoningEfforts: [], defaultEffort: null, supportsFast: false, fastDescription: null }], current: { model: 'model-id', reasoningEffort: null, fastMode: null }, source: '', warning: null });
await Promise.resolve();
const afterCatalog = await delayedCatalogBridge.getSnapshot();
assert.equal(afterCatalog.sharing?.composerByTask?.[selectedId]?.model.label, 'Late safe label', 'A late catalog result must update the existing cache for the next snapshot.');

const unavailableComposer = await createOperatorScopedBridge({ getSnapshot: async () => composerSource, sendMessage: async () => composerSource }, () => active).getSnapshot();
assert.deepEqual(unavailableComposer.sharing?.composerByTask?.[selectedId]?.context, { status: 'unavailable', reason: 'Context usage is unavailable until this model reports its context window.' });

// Attachment IDs are minted only by this visitor's scoped upload and are tied
// to the exact immutable share object and task; owner IDs never become usable.
let storedUploads = 0, attachmentSend: string[] = [];
active = grant();
const uploadBridge = createOperatorScopedBridge({
  getSnapshot: async () => source,
  sendMessage: async (_taskId, text, ids) => { sentText = text; attachmentSend = ids ?? []; return source; },
  storeAttachment: async (_target, file) => {
    storedUploads += 1;
    return { id: `33333333-3333-4333-8333-${String(storedUploads).padStart(12, '0')}`, name: file.filename, mimeType: file.mimeType, size: atob(file.dataBase64).length, path: '/private/new-upload' };
  },
}, () => active);
await assert.rejects(uploadBridge.storeAttachment!(selectedId, { filename: '../escape.txt', mimeType: 'text/plain', dataBase64: 'eA==' }), /valid file/i);
assert.equal(storedUploads, 0, 'Traversal must be rejected before native storage.');
await assert.rejects(uploadBridge.sendMessage(selectedId, 'owner ID', ['44444444-4444-4444-8444-444444444444']), /uploaded by this visitor/i);
const owned = await uploadBridge.storeAttachment!(selectedId, { filename: 'visitor.txt', mimeType: 'text/plain', dataBase64: 'eA==' });
await uploadBridge.sendMessage(selectedId, '', [owned.id]);
assert.deepEqual(attachmentSend, [owned.id]);
assert.ok(sentText.endsWith('@(Sam): '), 'Attachment-only delivery still uses the approved visitor identity.');
await assert.rejects(uploadBridge.sendMessage(selectedId, 'replay', [owned.id]), /uploaded by this visitor/i);
active = grant();
await assert.rejects(uploadBridge.sendMessage(selectedId, 'stale share', [owned.id]), /uploaded by this visitor/i);

let uploadNativeCalls = 0;
let releaseUpload!: () => void;
const uploadGate = new Promise<void>(resolve => { releaseUpload = resolve; });
active = grant();
const delayedUpload = createOperatorScopedBridge({
  getSnapshot: async () => { await uploadGate; return source; },
  sendMessage: async () => source,
  storeAttachment: async () => { uploadNativeCalls += 1; throw new Error('must not store'); },
}, () => active);
const uploadInFlight = delayedUpload.storeAttachment!(selectedId, { filename: 'waiting.txt', mimeType: 'text/plain', dataBase64: 'eA==' });
active = null;
releaseUpload();
await assert.rejects(uploadInFlight, /revoked|not shared/i);
assert.equal(uploadNativeCalls, 0, 'Revocation during upload authorization must prevent native storage.');

let release!: () => void;
const gate = new Promise<void>(resolve => { release = resolve; });
active = grant();
const delayed = createOperatorScopedBridge({
  getSnapshot: async () => { await gate; return source; },
  sendMessage: async () => { sent += 1; return source; },
}, () => active);
const inFlight = delayed.sendMessage(selectedId, 'must not escape');
active = null;
release();
await assert.rejects(inFlight, /revoked|not shared/i);
assert.equal(sent, 1, 'Revocation during the snapshot await must prevent dispatch.');

console.log('Operator sharing security assertions passed.');
