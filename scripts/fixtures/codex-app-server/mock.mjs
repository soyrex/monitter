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
let mcpEndpoint = null;
let mcpToken = null;
let mcpInitialized = false;

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

async function verifyMcp(method, params = {}) {
  if (process.env.MONITTER_FIXTURE_VERIFY_MCP_HTTP !== '1') return;
  if (!mcpEndpoint || !mcpToken) throw new Error('fixture missing HTTP MCP credentials');
  const headers = { authorization: `Bearer ${mcpToken}`, accept: 'application/json, text/event-stream', 'content-type': 'application/json' };
  if (mcpInitialized) headers['MCP-Protocol-Version'] = '2025-03-26';
  const result = await fetch(mcpEndpoint, { method: 'POST', headers, body: JSON.stringify({ jsonrpc: '2.0', id: Date.now(), method, params }) });
  if (!result.ok) throw new Error(`fixture MCP HTTP status ${result.status}`);
  const payload = await result.json();
  if (payload.error) throw new Error(`fixture MCP error ${payload.error.message}`);
  if (payload.result?.isError === true) throw new Error('fixture MCP tool returned an error result');
  if (method === 'initialize') mcpInitialized = true;
  return payload.result;
}

async function configureMcp(params) {
  const config = params?.config || {};
  if (typeof config['mcp_servers.monitter.url'] === 'string') {
    mcpEndpoint = config['mcp_servers.monitter.url'];
    mcpToken = process.env[config['mcp_servers.monitter.bearer_token_env_var'] || ''];
  }
  if (process.env.MONITTER_FIXTURE_VERIFY_MCP_HTTP === '1') {
    const init = await verifyMcp('initialize', { protocolVersion: '2025-03-26', capabilities: {}, clientInfo: { name: 'monitter-fixture', version: '1' } });
    const listing = await verifyMcp('tools/list');
    if (!init?.serverInfo || !Array.isArray(listing?.tools) || !listing.tools.some((tool) => tool.name === 'list_agents')) throw new Error('fixture MCP catalogue missing list_agents');
  }
}

async function beginTurn(id, params) {
  if (!userInputText(params?.input)) return rpcError(id, -32602, 'turn/start input must contain a text UserInput');
  if (process.env.MONITTER_FIXTURE_ERROR === '1') { rpcError(id, -32001, 'fixture injected protocol failure'); process.exitCode = 2; return; }
  try { await verifyMcp('tools/call', { name: 'list_agents', arguments: {} }); } catch (error) { return rpcError(id, -32002, error.message); }
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
  if (request.method === 'thread/start') return void configureMcp(request.params).then(() => response(request.id, { thread: thread(), model: 'fixture-model', modelProvider: 'openai', serviceTier: null, cwd: process.cwd(), instructionSources: [], approvalPolicy: 'on-request', approvalsReviewer: 'user', sandbox: { type: 'readOnly', networkAccess: false }, reasoningEffort: null })).catch((error) => rpcError(request.id, -32002, error.message));
  if (request.method === 'thread/resume') {
    const config = request.params?.config || {};
    const enabled = config['mcp_servers.monitter.enabled_tools'];
    const expectedTools = ['list_agents', 'delegate_task', 'send_message', 'get_task_result', 'wait_for_task', 'list_messages', 'cancel_delegation', 'terminal_run', 'skills_help', 'list_shared_skills', 'install_shared_skill'];
    if (request.params?.excludeTurns !== true || config['mcp_servers.monitter.required'] !== true || typeof config['mcp_servers.monitter.url'] !== 'string' || !config['mcp_servers.monitter.url'].startsWith('http://') || config['mcp_servers.monitter.bearer_token_env_var'] !== 'MONITTER_TOKEN' || config['mcp_servers.monitter.command'] !== undefined || config['mcp_servers.monitter.args'] !== undefined || JSON.stringify(enabled) !== JSON.stringify(expectedTools) || JSON.stringify(config).includes('fixture-token')) {
      return rpcError(request.id, -32602, 'fixture requires HTTP Monitter MCP config, env bearer token, and exact tool allowlist');
    }
    const result = () => response(request.id, { thread: thread(request.params?.threadId || threadId), model: 'fixture-model', modelProvider: 'openai', serviceTier: null, cwd: process.cwd(), instructionSources: [], approvalPolicy: 'on-request', approvalsReviewer: 'user', sandbox: { type: 'readOnly', networkAccess: false }, reasoningEffort: null, turnsBackwardsCursor: null, itemsBackwardsCursor: null });
    if (process.env.MONITTER_FIXTURE_VERIFY_MCP_HTTP === '1') return void configureMcp(request.params).then(result).catch((error) => rpcError(request.id, -32002, error.message));
    const delay = Number(process.env.MONITTER_FIXTURE_DELAY_THREAD_RESUME_MS || 0);
    return delay > 0 ? setTimeout(result, delay) : result();
  }
  if (request.method === 'turn/start') return void beginTurn(request.id, request.params);
  if (request.method === 'turn/steer') {
    if (!activeTurn) return rpcError(request.id, -32602, 'No active fixture turn');
    if (request.params?.threadId !== threadId || request.params?.expectedTurnId !== turnId || !userInputText(request.params?.input)) {
      return rpcError(request.id, -32602, 'fixture requires the active thread, turn, and text input');
    }
    return response(request.id, { turnId });
  }
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
