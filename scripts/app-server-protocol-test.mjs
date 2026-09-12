#!/usr/bin/env node

// Shape/lifecycle regression test for the local fixture. No Codex account,
// auth, network, model, Tauri app, or paid service is touched.
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { strict as assert } from 'node:assert';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const fixture = path.join(path.dirname(fileURLToPath(import.meta.url)), 'fixtures/codex-app-server/mock.mjs');
const child = spawn(process.execPath, [fixture], { stdio: ['pipe', 'pipe', 'inherit'] });
const lines = []; child.stdout.setEncoding('utf8');
child.stdout.on('data', (chunk) => lines.push(...chunk.trim().split('\n').filter(Boolean).map((line) => JSON.parse(line))));
const waitFor = async (predicate) => { for (;;) { const found = lines.find(predicate); if (found) return found; await new Promise((resolve) => setTimeout(resolve, 5)); } };
const call = (id, method, params) => child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`);

call(1, 'initialize', { clientInfo: { name: 'test', title: null, version: '1' }, capabilities: { experimentalApi: false, requestAttestation: false } });
assert.equal((await waitFor((m) => m.id === 1)).result.platformOs, 'macos');
call(2, 'thread/start', {}); const started = await waitFor((m) => m.id === 2); assert.equal(started.result.thread.historyMode, 'paginated');
call(3, 'turn/start', { threadId: started.result.thread.id, input: [{ type: 'text', text: 'fixture', text_elements: [] }] });
assert.equal((await waitFor((m) => m.id === 3)).result.turn.status, 'inProgress');
const approval = await waitFor((m) => m.method === 'item/commandExecution/requestApproval');
assert.deepEqual(Object.keys(approval.params).sort(), ['approvalId','command','commandActions','cwd','environmentId','itemId','kind','proposedExecpolicyAmendment','proposedNetworkPolicyAmendments','reason','startedAtMs','threadId','turnId']);
call(approval.id, 'response', { decision: 'accept' });
const fileApproval = await waitFor((m) => m.method === 'item/fileChange/requestApproval');
assert.equal(fileApproval.params.threadId, started.result.thread.id);
call(fileApproval.id, 'response', { decision: 'accept' });
const userInput = await waitFor((m) => m.method === 'item/tool/requestUserInput');
assert.equal(userInput.params.isBlocking, true);
call(userInput.id, 'response', { answers: { confirm: { answers: ['yes'] } } });
assert.equal((await waitFor((m) => m.method === 'turn/completed')).params.turn.status, 'completed');
call(4, 'turn/interrupt', { threadId: started.result.thread.id, turnId: 'no-active-turn' });
assert.equal((await waitFor((m) => m.id === 4)).error.code, -32602);
child.kill(); await once(child, 'close');
console.log('app-server protocol fixture: ok');
