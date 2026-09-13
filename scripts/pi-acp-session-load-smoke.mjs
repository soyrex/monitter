// Optional offline compatibility proof for an already-installed pi-acp. This
// exercises the real ACP adapter, but ONLY a fake Pi subprocess/private store.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtempSync, readFileSync, realpathSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = dirname(fileURLToPath(import.meta.url));
const entryArgument = process.argv[2];
if (!entryArgument) throw new Error('Usage: node scripts/pi-acp-session-load-smoke.mjs /absolute/path/to/pi-acp');
const entry = realpathSync(resolve(entryArgument));
const scratch = mkdtempSync(join(tmpdir(), 'monitter-pi-acp-smoke-'));
const children = new Set();

function connection() {
  const child = spawn(process.execPath, [join(root, 'fixtures/pi-acp-isolated-wrapper.mjs')], {
    cwd: scratch, detached: true, stdio: ['pipe', 'pipe', 'pipe'],
    env: { ...process.env, MONITTER_PI_SMOKE_DIR: scratch, MONITTER_PI_ACP_ENTRY: entry,
      PI_CODING_AGENT_DIR: join(scratch, 'agent'), PI_ACP_PI_COMMAND: join(root, 'fixtures/pi-rpc-session-fixture.mjs') },
  });
  children.add(child);
  let nextId = 1, buffer = '', stderr = '';
  const pending = new Map(), notifications = [];
  const closed = new Promise(resolve => child.once('close', resolve));
  child.closedPromise = closed;
  child.on('error', error => {
    for (const request of pending.values()) { clearTimeout(request.timer); request.reject(error); }
    pending.clear();
  });
  child.stdin.on('error', error => {
    for (const request of pending.values()) { clearTimeout(request.timer); request.reject(error); }
    pending.clear();
  });
  child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-4000); });
  child.stdout.on('data', bytes => {
    buffer += bytes;
    for (;;) {
      const newline = buffer.indexOf('\n');
      if (newline < 0) break;
      const frame = JSON.parse(buffer.slice(0, newline));
      buffer = buffer.slice(newline + 1);
      if (frame.id != null && pending.has(frame.id)) {
        const request = pending.get(frame.id); pending.delete(frame.id); clearTimeout(request.timer);
        if (frame.error) request.reject(new Error(JSON.stringify(frame.error)));
        else request.resolve(frame.result);
      } else notifications.push(frame);
    }
  });
  child.once('close', () => {
    for (const request of pending.values()) { clearTimeout(request.timer); request.reject(new Error(`Adapter exited: ${stderr}`)); }
    pending.clear();
  });
  return {
    notifications,
    request(method, params) {
      return new Promise((resolve, reject) => {
        const id = nextId++;
        const timer = setTimeout(() => { pending.delete(id); reject(new Error(`Timed out: ${method}`)); }, 10000);
        pending.set(id, { resolve, reject, timer });
        child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
      });
    },
    async stop() {
      try { process.kill(-child.pid, 'SIGTERM'); } catch (error) { if (error.code !== 'ESRCH') throw error; }
      const timer = setTimeout(() => { try { process.kill(-child.pid, 'SIGKILL'); } catch {} }, 2000);
      await closed; clearTimeout(timer); children.delete(child);
    },
  };
}

try {
  const initialize = { protocolVersion: 1, clientInfo: { name: 'monitter-offline-smoke', version: '1' }, clientCapabilities: {} };
  const first = connection();
  assert.equal((await first.request('initialize', initialize)).agentCapabilities.loadSession, true);
  const original = await first.request('session/new', { cwd: scratch, mcpServers: [] });
  assert.equal(original.sessionId, '11111111-2222-4333-8444-555555555555');
  await first.stop();
  const second = connection();
  await second.request('initialize', initialize);
  await second.request('session/load', { sessionId: original.sessionId, cwd: scratch, mcpServers: [] });
  const updates = second.notifications.filter(frame => frame.method === 'session/update');
  assert(updates.some(frame => frame.params.update.content?.text === 'Fixture saved question'));
  assert(updates.some(frame => frame.params.update.content?.text === 'Fixture saved reply'));
  await second.stop();
  const calls = readFileSync(join(scratch, 'rpc-log.jsonl'), 'utf8').trim().split('\n').map(JSON.parse);
  assert.equal(calls.filter(call => call.type === 'spawn').length, 2);
  assert(calls.filter(call => call.type === 'spawn')[1].args.includes('--session'));
  assert(!calls.some(call => call.type === 'prompt'));
  console.log('PASS: installed pi-acp reloads the same saved session on a fresh process without a prompt, model call, or user data access.');
} finally {
  for (const child of children) { try { process.kill(-child.pid, 'SIGKILL'); } catch {} }
  await Promise.all([...children].map(child => child.closedPromise));
  rmSync(scratch, { recursive: true, force: true });
}
