#!/usr/bin/env node

// Deterministic, local-only Codex app-server protocol fixture. It intentionally
// implements only the lifecycle used by Monitter's adapter tests.
import readline from 'node:readline';

const threadId = '00000000-0000-7000-8000-000000000001';
let turnId = '00000000-0000-7000-8000-000000000002';
let itemId = '00000000-0000-7000-8000-000000000003';
let turnNumber = 0;
let activeTurn = false;
let requestNumber = 100;
let pendingRequestKind = null;

const send = (message) => process.stdout.write(`${JSON.stringify(message)}\n`);
const response = (id, result) => send({ jsonrpc: '2.0', id, result });
const notification = (method, params) => send({ jsonrpc: '2.0', method, params });
const rpcError = (id, code, message) => send({ jsonrpc: '2.0', id, error: { code, message } });

const thread = (id = threadId) => ({
  id, sessionId: id, forkedFromId: null, parentThreadId: null, preview: 'Protocol fixture thread',
  ephemeral: true, section: null, sectionEnteredAt: null, projectId: null, historyMode: 'paginated',
  modelProvider: 'openai', model: 'fixture-model', reasoningEffort: null,
  createdAt: 1726000000, updatedAt: 1726000000, recencyAt: 1726000000,
  status: { type: 'idle' }, path: null, cwd: process.cwd(), cliVersion: '0.154.0-fixture',
  originator: 'monitter-fixture', source: 'appServer', threadSource: null,
  agentNickname: null, agentRole: null, gitInfo: null, name: 'Protocol fixture', turns: [],
});

const turn = (status = 'inProgress') => ({ id: turnId, items: [], status, startedAt: 1726000000, completedAt: status === 'inProgress' ? null : 1726000001, durationMs: status === 'inProgress' ? null : 1000, error: null, itemsView: 'full' });
const userInputText = (input) => Array.isArray(input) && input.some((entry) => entry?.type === 'text');

function beginTurn(id, params) {
  if (!userInputText(params?.input)) return rpcError(id, -32602, 'turn/start input must contain a text UserInput');
  if (process.env.MONITTER_FIXTURE_ERROR === '1') { rpcError(id, -32001, 'fixture injected protocol failure'); process.exitCode = 2; return; }
  turnNumber += 1;
  turnId = `00000000-0000-7000-8000-00000000000${turnNumber + 1}`;
  itemId = `00000000-0000-7000-8000-00000000000${turnNumber + 2}`;
  activeTurn = true;
  response(id, { turn: turn() });
  notification('turn/started', { threadId, turn: turn() });
  notification('item/agentMessage/delta', { threadId, turnId, itemId, delta: 'fixture response' });
  notification('item/completed', { threadId, turnId, completedAtMs: 1726000000500, item: {
    type: 'mcpToolCall', id: `${itemId}-mcp`, server: 'fixture', tool: 'image', status: 'completed',
    arguments: {}, appContext: null, pluginId: null, readOnlyHint: true, durationMs: 1,
    result: { content: [{ type: 'image', data: '/9j/2Q==', mimeType: 'image/jpeg' }], structuredContent: null, _meta: null }, error: null,
  }});
  // Server requests exercise the adapter's exact request/response routing.
  const approvalId = ++requestNumber;
  pendingRequestKind = 'command';
  send({ jsonrpc: '2.0', id: approvalId, method: 'item/commandExecution/requestApproval', params: {
    kind: 'command', threadId, turnId, itemId, startedAtMs: 1726000000000,
    approvalId: null, environmentId: null, reason: 'fixture approval', command: 'printf fixture', cwd: process.cwd(),
    commandActions: null, proposedExecpolicyAmendment: null, proposedNetworkPolicyAmendments: null,
  }});
}

const rl = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
rl.on('line', (line) => {
  let request;
  try { request = JSON.parse(line); } catch { return rpcError(null, -32700, 'Invalid JSON'); }
  if (request.method === 'initialize') return response(request.id, { userAgent: 'monitter-protocol-fixture', codexHome: process.cwd(), platformFamily: 'unix', platformOs: 'macos' });
  if (request.method === 'initialized') return;
  if (request.method === 'thread/start') return response(request.id, { thread: thread(), model: 'fixture-model', modelProvider: 'openai', serviceTier: null, cwd: process.cwd(), instructionSources: [], approvalPolicy: 'on-request', approvalsReviewer: 'user', sandbox: { type: 'readOnly', networkAccess: false }, reasoningEffort: null });
  if (request.method === 'thread/resume') return response(request.id, { thread: thread(request.params?.threadId || threadId), model: 'fixture-model', modelProvider: 'openai', serviceTier: null, cwd: process.cwd(), instructionSources: [], approvalPolicy: 'on-request', approvalsReviewer: 'user', sandbox: { type: 'readOnly', networkAccess: false }, reasoningEffort: null, turnsBackwardsCursor: null, itemsBackwardsCursor: null });
  if (request.method === 'turn/start') return beginTurn(request.id, request.params);
  if (request.method === 'turn/interrupt') {
    if (!activeTurn) return rpcError(request.id, -32602, 'No active fixture turn');
    activeTurn = false; response(request.id, {}); notification('turn/completed', { threadId, turn: turn('interrupted') }); return;
  }
  if (request.method === 'item/commandExecution/requestApproval' || request.method === 'item/fileChange/requestApproval' || request.method === 'item/tool/requestUserInput' || request.method === 'mcpServer/elicitation/request') {
    response(request.id, {}); return;
  }
  // Any response to our approval request completes the deterministic turn.
  if (request.id >= 100) {
    if (pendingRequestKind === 'command' || pendingRequestKind === 'file') {
      if (!request.result?.decision || !['accept', 'decline', 'cancel'].includes(request.result.decision)) {
        rpcError(request.id, -32602, 'fixture requires decision accept, decline, or cancel'); return;
      }
    } else if (pendingRequestKind === 'input') {
      if (!request.result?.answers || typeof request.result.answers !== 'object') {
        rpcError(request.id, -32602, 'fixture requires an answers object'); return;
      }
    } else { rpcError(request.id, -32602, 'fixture has no pending request'); return; }
    if (pendingRequestKind === 'command') {
      pendingRequestKind = 'file'; const nextId = ++requestNumber;
      send({ jsonrpc: '2.0', id: nextId, method: 'item/fileChange/requestApproval', params: { threadId, turnId, itemId, startedAtMs: 1726000000000, reason: 'fixture file approval', grantRoot: null } });
    } else if (pendingRequestKind === 'file') {
      pendingRequestKind = 'input'; const nextId = ++requestNumber;
      send({ jsonrpc: '2.0', id: nextId, method: 'item/tool/requestUserInput', params: { threadId, turnId, itemId, questions: [{ id: 'confirm', header: 'Confirm', question: 'Continue fixture?', isOther: false, isSecret: false, options: null }], isBlocking: true, autoResolutionMs: null } });
    } else {
      pendingRequestKind = null;
      notification('item/completed', { item: { type: 'agentMessage', id: itemId, text: 'fixture response', phase: null, memoryCitation: null, delivery: null, questions: null }, threadId, turnId, completedAtMs: 1726000001000 });
      activeTurn = false; notification('turn/completed', { threadId, turn: turn('completed') });
    }
    return;
  }
  rpcError(request.id, -32601, `Unsupported fixture method: ${request.method}`);
});
