#!/usr/bin/env node
// ACP v1 fallback for the installed Google Antigravity CLI.  It deliberately
// does not translate AGY tool permissions: AGY's own settings remain authority.
import { spawn } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import readline from 'node:readline';

const MAX_FRAME = 1024 * 1024;
const MAX_AGY_FRAME = 16 * 1024 * 1024;
const MAX_TURN_OUTPUT = 64 * 1024 * 1024;
const MAX_TEXT = 512 * 1024;
const MAX_TOOL_FIELD = 64 * 1024;
const [, , ...argv] = process.argv;
let agy = process.env.MONITTER_AGY_COMMAND || 'agy';
const permission = process.env.MONITTER_AGY_PERMISSION === 'yolo' ? 'yolo' : 'accept-edits';
for (let i = 0; i < argv.length; i += 1) {
  if (argv[i] === '--agy' && argv[i + 1]) { agy = argv[++i]; continue; }
  if (argv[i] === '--help') {
    process.stdout.write('Usage: agy-acp-bridge.mjs [--agy /absolute/path/to/agy]\n');
    process.exit(0);
  }
  throw new Error('Only --agy /absolute/path/to/agy is accepted.');
}
if (agy.includes('\0')) throw new Error('Invalid AGY executable path.');

let initialized = false;
let session = null;
let current = null;

function send(frame) { process.stdout.write(`${JSON.stringify(frame)}\n`); }
function reply(id, result) { send({ jsonrpc: '2.0', id, result }); }
function fail(id, code, message) { send({ jsonrpc: '2.0', id, error: { code, message } }); }
function note(sessionId, update) { send({ jsonrpc: '2.0', method: 'session/update', params: { sessionId, update } }); }
function safeMessage(message) { return String(message).replace(/[\x00-\x1f\x7f]/g, ' ').slice(0, 240); }
function boundedToolField(value) {
  if (value === undefined) return undefined;
  const serialized = JSON.stringify(value);
  return serialized?.length <= MAX_TOOL_FIELD ? value : '[AGY tool detail truncated]';
}

function promptText(prompt) {
  if (!Array.isArray(prompt) || prompt.length === 0) throw new Error('ACP prompt must contain text.');
  let text = '';
  for (const block of prompt) {
    if (!block || block.type !== 'text' || typeof block.text !== 'string') throw new Error('AGY bridge supports ACP text prompt blocks only.');
    text += block.text;
    if (text.length > MAX_TEXT) throw new Error('ACP prompt exceeds 512 KiB.');
  }
  return text;
}

function stop(error) {
  const active = session;
  session = null;
  if (active?.child && !active.child.killed) active.child.kill('SIGTERM');
  if (active?.requestId !== null && active?.requestId !== undefined) fail(active.requestId, -32603, error);
  if (current) { fail(current.id, -32800, error); current = null; }
}

function startSession({ cwd, conversationId, requestId }) {
  if (session) throw new Error('An ACP session is already active in this bridge process.');
  const args = ['--input-format', 'stream-json', '--output-format', 'stream-json'];
  args.push('--mode=accept-edits');
  if (permission === 'yolo') args.push('--dangerously-skip-permissions');
  if (conversationId) args.push('--conversation', conversationId);
  const child = spawn(agy, args, { cwd: typeof cwd === 'string' && cwd ? cwd : process.cwd(), stdio: ['pipe', 'pipe', 'pipe'], shell: false });
  const next = { child, sessionId: conversationId || null, requestId, pending: Buffer.alloc(0), output: 0, ready: false, closed: false, turn: 0, tools: new Set() };
  session = next;
  child.on('error', () => { if (session === next) stop('Could not launch AGY. Check the configured executable and existing sign-in.'); });
  child.stderr.on('data', () => {}); // Drain authentication diagnostics; never relay them.
  child.stdout.on('data', chunk => ingestAgy(next, chunk));
  child.on('exit', code => {
    if (session !== next || next.closed) return;
    next.closed = true;
    if (!next.ready) stop('AGY exited before its stream initialized. Check AGY sign-in and configuration.');
    else if (current) stop(`AGY ended before the turn completed (exit ${Number.isInteger(code) ? code : 'unknown'}).`);
    else session = null;
  });
}

function ingestAgy(state, chunk) {
  if (session !== state) return;
  state.output += chunk.length;
  if (state.output > MAX_TURN_OUTPUT) return stop('AGY output exceeded the 64 MiB turn limit.');
  state.pending = Buffer.concat([state.pending, chunk]);
  for (;;) {
    const index = state.pending.indexOf(10);
    if (index < 0) break;
    if (index > MAX_AGY_FRAME) return stop('AGY emitted an oversized stream frame.');
    const line = state.pending.subarray(0, index); state.pending = state.pending.subarray(index + 1);
    if (!line.toString('utf8').trim()) continue;
    let event;
    try { event = JSON.parse(line.toString('utf8')); } catch { return stop('AGY emitted invalid stream JSON.'); }
    if (event.event === 'init') {
      const id = event.conversation_id;
      if (typeof id !== 'string' || !id || id.length > 256) return stop('AGY did not provide a safe conversation ID.');
      if (state.sessionId && state.sessionId !== id) return stop('AGY resumed a different conversation than requested.');
      state.sessionId = id; state.ready = true;
      if (state.requestId !== null) { reply(state.requestId, { sessionId: id }); state.requestId = null; }
      continue;
    }
    if (!state.ready) return stop('AGY sent stream data before initialization.');
    if (event.event === 'step_update' && event.step_update?.step_type === 'tool' && current) {
      const step = event.step_update;
      const index = Number.isSafeInteger(step.step_index) && step.step_index >= 0 ? step.step_index : null;
      if (index === null) continue;
      const toolCallId = `agy-${state.turn}-${index}`;
      const first = !state.tools.has(toolCallId);
      state.tools.add(toolCallId);
      const update = {
        sessionUpdate: first ? 'tool_call' : 'tool_call_update',
        toolCallId,
        title: safeMessage(step.tool_name || step.tool_info?.name || 'AGY tool'),
        status: step.state === 'DONE' ? 'completed' : 'pending',
      };
      const rawInput = boundedToolField(step.tool_info?.parameters);
      const rawOutput = boundedToolField(step.tool_info?.output);
      if (rawInput !== undefined) update.rawInput = rawInput;
      if (rawOutput !== undefined) update.rawOutput = rawOutput;
      note(state.sessionId, update);
    } else if (event.event === 'step_update' && event.step_update?.step_type === 'agent_response' && typeof event.step_update.text_delta === 'string' && current) {
      const text = event.step_update.text_delta;
      if (text.length > MAX_TEXT) return stop('AGY emitted an oversized text update.');
      current.text += text;
      if (current.text.length > MAX_TEXT) return stop('AGY response exceeded 512 KiB.');
      note(state.sessionId, { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text } });
    } else if (event.event === 'result' && current) {
      const result = event.result || {};
      const request = current; current = null; state.output = 0;
      if (result.status !== 'SUCCESS') fail(request.id, -32603, safeMessage(result.error || `AGY returned ${result.status || 'an unsuccessful result'}.`));
      else {
        if (typeof result.response === 'string' && result.response.startsWith(request.text)) {
          const remaining = result.response.slice(request.text.length);
          if (remaining) note(state.sessionId, { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: remaining.slice(0, MAX_TEXT) } });
        }
        reply(request.id, { stopReason: 'end_turn' });
      }
    }
  }
  if (state.pending.length > MAX_AGY_FRAME) stop('AGY emitted an oversized stream frame.');
}

function handle(frame) {
  if (!frame || frame.jsonrpc !== '2.0') return;
  const { id, method, params = {} } = frame;
  if (method === 'initialize') {
    if (initialized) return fail(id, -32600, 'ACP is already initialized.');
    if (params.protocolVersion !== 1) return fail(id, -32602, 'This bridge supports ACP protocol version 1 only.');
    initialized = true;
    return reply(id, { protocolVersion: 1, agentInfo: { name: 'Google AGY headless fallback', version: '1' }, agentCapabilities: { loadSession: true } });
  }
  if (!initialized) return id !== undefined && fail(id, -32002, 'ACP initialize is required first.');
  if (method === 'initialized') return;
  if (method === 'session/new') {
    try { startSession({ cwd: params.cwd, conversationId: null, requestId: id }); } catch (error) { fail(id, -32603, safeMessage(error.message)); }
    return;
  }
  if (method === 'session/load') {
    if (typeof params.sessionId !== 'string' || !params.sessionId || params.sessionId.length > 256) return fail(id, -32602, 'session/load requires the AGY conversation ID returned by this bridge.');
    try { startSession({ cwd: params.cwd, conversationId: params.sessionId, requestId: id }); } catch (error) { fail(id, -32603, safeMessage(error.message)); }
    return;
  }
  if (method === 'session/prompt') {
    if (!session?.ready || params.sessionId !== session.sessionId) return fail(id, -32001, 'Unknown or not-ready AGY session.');
    if (current) return fail(id, -32000, 'AGY accepts one prompt at a time.');
    try {
      const message = promptText(params.prompt);
      current = { id, text: '' }; session.output = 0; session.turn += 1; session.tools.clear();
      session.child.stdin.write(`${JSON.stringify({ event: 'user', message: { content: message } })}\n`);
    } catch (error) { fail(id, -32602, safeMessage(error.message)); }
    return;
  }
  if (method === 'session/cancel') {
    if (session && (!params.sessionId || params.sessionId === session.sessionId)) stop('AGY turn cancelled by client.');
    if (id !== undefined) reply(id, {});
    return;
  }
  if (id !== undefined) fail(id, -32601, 'Unsupported ACP method. AGY headless has no bridgeable approval callbacks.');
}

const input = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
input.on('line', line => {
  if (Buffer.byteLength(line) > MAX_FRAME) return stop('ACP input frame exceeded 1 MiB.');
  try { handle(JSON.parse(line)); } catch { /* Malformed input has no trustworthy request id. */ }
});
input.on('close', () => { if (session?.child && !session.child.killed) session.child.kill('SIGTERM'); });
