import { mount } from 'svelte';
import { setOwnerRelayInvoker } from '$lib/lan';
import Harness from './owner-relay-app-history-harness.svelte';

const taskId = '11111111-1111-4111-8111-111111111111';
const messageId = (n: number) => `22222222-2222-4222-8222-${String(n).padStart(12, '0')}`;
let epoch = 'epoch-a', revision = 1;
let mode = '';
let resetCount = 0;
let pageCalls = 0;
let rows = Array.from({ length: 100 }, (_, n) => ({ id: messageId(n + 1), taskId, role: n % 2 ? 'assistant' : 'user', text: `app-history-${n + 1}`, createdAt: n + 1, attachments: [] }));
const task = { id: taskId, agentId: 'main', title: 'History regression', nativeSessionId: null, archived: false, status: 'completed', createdAt: 1, updatedAt: 1, parentTaskId: null, channelId: null, projectId: null, hostId: 'local', cwd: '/tmp', provider: 'codex', model: '', sandbox: 'read-only' };
const metadata = () => ({ hosts: [], agents: [{ id: 'main', name: 'Fixture agent', provider: 'codex', hostId: 'local', cwd: '/tmp', expertise: [], responsibilities: [], skills: [], color: '#397e61', sandbox: 'read-only', instructions: '', description: '', model: '' }], tasks: [task], messages: [], events: [], channels: [], projects: [], collaborations: [], queuedMessages: [], approvalRequests: [], approvalRules: [], settings: { accent: '#397e61', theme: 'light', interfaceScale: 100, sidebarView: 'standard', showToolActivity: true, showReasoningSummaries: true } });
const journal: { revision: number; messages: typeof rows; removed: string[] }[] = [];
const rev = () => `${epoch}:${revision}`;
function advance(messages: typeof rows = [], removed: string[] = []) { revision++; journal.push({ revision, messages, removed }); }
setOwnerRelayInvoker(async (command, args) => {
  if (command === 'get_command_capabilities') return { protocolVersion: 1, commands: ['get_ui_delta', 'get_task_messages'] };
  if (command === 'get_ui_delta') {
    // Simulate unrelated live agents advancing the global workspace on every read.
    advance();
    const from = String(args.revision ?? '');
    if (!from.startsWith(`${epoch}:`)) { resetCount++; return { revision: rev(), snapshot: metadata(), delta: null }; }
    const changes = journal.filter(change => change.revision > Number(from.split(':')[1]));
    return { revision: rev(), snapshot: null, delta: { fromRevision: from, metadata: metadata(), messages: changes.flatMap(change => change.messages), removedMessageIds: changes.flatMap(change => change.removed), retainedChannelIds: [], retainedSubagentTranscriptIds: [] } };
  }
  if (command === 'get_task_messages') {
    pageCalls++;
    advance();
    const end = args.beforeId ? rows.findIndex(row => row.id === args.beforeId) : rows.length;
    if (end < 0) throw new Error('Unknown history cursor');
    const start = Math.max(0, end - Number(args.limit ?? 64));
    const messages = structuredClone(rows.slice(start, end));
    const page = { revision: rev(), messages, nextBeforeId: start ? messages[0]?.id : null };
    if (mode === 'edit-remove') {
      mode = '';
      const removed = messages[0].id;
      const updated = { ...messages[1], text: 'app-history-edited-after-page' };
      rows = rows.filter(row => row.id !== removed).map(row => row.id === updated.id ? updated : row);
      advance([updated], [removed]);
    } else if (mode === 'remove-page') {
      mode = '';
      const removed = messages.map(message => message.id);
      const ids = new Set(removed);
      rows = rows.filter(row => !ids.has(row.id));
      advance([], removed);
    } else if (mode === 'reset') {
      mode = '';
      epoch = 'epoch-b'; revision = 1; journal.length = 0;
      rows = rows.filter(row => row.id !== messages[0].id).map(row => row.id === messages.at(-1)?.id ? { ...row, text: 'app-history-after-reset' } : row);
    } else if (mode === 'wrong-task') {
      mode = '';
      page.messages[0].taskId = '33333333-3333-4333-8333-333333333333';
    }
    return page;
  }
  throw new Error(`Fixture rejects unrelated command: ${command}`);
});
(window as any).__historyServer = { taskId, messageId, mode: (value: string) => mode = value, counts: () => ({ pageCalls, resetCount }) };
mount(Harness, { target: document.getElementById('app')! });
