import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';

// Execute the production guard and generated inventory without a browser or IPC.
const moduleUrl = code => `data:text/javascript;base64,${Buffer.from(code).toString('base64')}`;
const compile = path => ts.transpileModule(readFileSync(path, 'utf8'), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
}).outputText;
const generatedUrl = moduleUrl(compile('src/lib/generated-command-contract.ts'));
const { createCommandGuard } = await import(moduleUrl(compile('src/lib/command-protocol.ts')
  .replace("'./generated-command-contract'", JSON.stringify(generatedUrl))));
const { COMMAND_CONTRACT_PROTOCOL_VERSION: version } = await import(generatedUrl);
const available = commands => ({ protocolVersion: version, commands });
let assertions = 0;
const rejects = async (guard, command, pattern) => { await assert.rejects(guard(command), pattern); assertions++; };

let calls = 0;
let capabilities = available(['cancel_task']);
const guard = createCommandGuard(async () => { calls++; return capabilities; });
await guard('cancel_task');
assert.equal(calls, 1); assertions++;
await rejects(guard, 'remove_task', /does not support/);
// A desktop replacement cannot inherit capabilities cached for a previous backend.
capabilities = { protocolVersion: version + 1, commands: ['cancel_task'] };
await rejects(guard, 'cancel_task', /Unsupported Monitter command protocol/);
capabilities = available(['cancel_task']);
await guard('cancel_task');
assert.equal(calls, 4); assertions++;

for (const response of [null, {}, available([23]), { protocolVersion: version }]) {
  await rejects(createCommandGuard(async () => response), 'cancel_task', /invalid command/);
}
for (const message of ['network disconnected', 'permission denied', 'unknown command cancel_task']) {
  await rejects(createCommandGuard(async () => { throw new Error(message); }), 'cancel_task', new RegExp(message));
}
for (const message of [
  'Command get_command_capabilities not found',
  'unknown command: get_command_capabilities',
  'unknown invoke command get_command_capabilities',
  'get_command_capabilities is not available',
]) {
  const legacy = createCommandGuard(async () => { throw new Error(message); });
  await legacy('cancel_task');
  await legacy('send_message');
  await rejects(legacy, 'get_ui_delta', /does not support/);
  await rejects(legacy, 'invented_mutation', /does not support/);
}
let resolveProbe;
let concurrentCalls = 0;
const concurrent = createCommandGuard(() => { concurrentCalls++; return new Promise(resolve => { resolveProbe = resolve; }); });
const waiting = [concurrent('cancel_task'), concurrent('send_message')];
assert.equal(concurrentCalls, 1); assertions++;
resolveProbe(available(['cancel_task', 'send_message']));
await Promise.all(waiting);

// Compatibility checks must precede dispatch; uncertain dispatch failures are never retried.
let dispatched = 0;
const dispatch = async commandGuard => {
  await commandGuard('cancel_task');
  dispatched++;
  throw new Error('delivery unknown');
};
await assert.rejects(dispatch(createCommandGuard(async () => ({ protocolVersion: 999, commands: ['cancel_task'] }))), /Unsupported/);
assert.equal(dispatched, 0); assertions++;
await assert.rejects(dispatch(createCommandGuard(async () => available(['cancel_task']))), /delivery unknown/);
assert.equal(dispatched, 1); assertions++;

const bootstrap = createCommandGuard(async () => { throw new Error('must not probe'); });
await bootstrap('get_command_capabilities');
await bootstrap('use_packaged_ui');
assertions += 2;
console.log(`Command compatibility guard passed ${assertions} assertions (no IPC or mutations).`);
