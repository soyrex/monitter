import assert from 'node:assert/strict';
import {
  createOperatorScopedBridge,
  formatOperatorMessage,
  sharedSnapshot,
  splitOperatorMessage,
  validOperatorName,
  type ActiveOperatorShare,
} from '../src/lib/operator-sharing.ts';
import type { Snapshot, Task } from '../src/lib/types.ts';

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
assert.deepEqual(visitor.messages[0].attachments, []);
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

let active: ActiveOperatorShare | null = grant();
let sent = 0;
let sentText = '';
const bridge = createOperatorScopedBridge({
  getSnapshot: async () => source,
  sendMessage: async (_taskId, text) => { sent += 1; sentText = text; return source; },
}, () => active);
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
