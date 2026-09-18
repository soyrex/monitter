import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const mock = new URL('./mock.mjs', import.meta.url);
const timeoutMs = 3_000;

function startFixture(environment = {}) {
  const fixtureEnvironment = Object.fromEntries(
    Object.entries(process.env).filter(([key]) => !key.startsWith('MONITTER_FIXTURE_')),
  );
  const child = spawn(process.execPath, [fileURLToPath(mock)], {
    env: { ...fixtureEnvironment, ...environment },
    stdio: ['pipe', 'pipe', 'pipe'],
  });
  const messages = [];
  const waiters = [];
  let stderr = '';
  let stopping = false;

  const rejectWaiters = error => {
    for (const waiter of waiters.splice(0)) {
      clearTimeout(waiter.timer);
      waiter.reject(error);
    }
  };

  const deliver = message => {
    messages.push(message);
    for (const waiter of [...waiters]) {
      if (!waiter.predicate(message)) continue;
      clearTimeout(waiter.timer);
      waiters.splice(waiters.indexOf(waiter), 1);
      waiter.resolve(message);
    }
  };
  let buffered = '';
  child.stdout.setEncoding('utf8');
  child.stdout.on('data', chunk => {
    buffered += chunk;
    for (;;) {
      const newline = buffered.indexOf('\n');
      if (newline < 0) return;
      const line = buffered.slice(0, newline);
      buffered = buffered.slice(newline + 1);
      try {
        deliver(JSON.parse(line));
      } catch (error) {
        rejectWaiters(new Error(`Fixture emitted invalid JSON: ${error.message}`));
      }
    }
  });
  child.stderr.setEncoding('utf8');
  child.stderr.on('data', chunk => { stderr += chunk; });
  child.once('error', error => {
    if (!stopping) rejectWaiters(new Error(`Fixture process failed: ${error.message}`));
  });
  child.once('exit', (code, signal) => {
    if (!stopping) rejectWaiters(new Error(`Fixture exited early (${code ?? signal}). stderr: ${stderr}`));
  });

  return {
    send(message) { child.stdin.write(`${JSON.stringify(message)}\n`); },
    waitFor(predicate) {
      const existing = messages.find(predicate);
      if (existing) return Promise.resolve(existing);
      return new Promise((resolve, reject) => {
        const waiter = { predicate, resolve, timer: setTimeout(() => {
          waiters.splice(waiters.indexOf(waiter), 1);
          reject(new Error(`Fixture response timed out. stderr: ${stderr}`));
        }, timeoutMs) };
        waiters.push(waiter);
      });
    },
    all() { return [...messages]; },
    async stop() {
      stopping = true;
      if (child.exitCode !== null || child.signalCode !== null) return;
      child.kill();
      await new Promise(resolve => child.once('exit', resolve));
    },
  };
}

async function startTurn(fixture) {
  fixture.send({ jsonrpc: '2.0', id: 1, method: 'initialize', params: {} });
  await fixture.waitFor(message => message.id === 1 && message.result);
  fixture.send({ jsonrpc: '2.0', method: 'initialized', params: {} });
  fixture.send({ jsonrpc: '2.0', id: 2, method: 'thread/start', params: { config: {} } });
  await fixture.waitFor(message => message.id === 2 && message.result);
  fixture.send({
    jsonrpc: '2.0', id: 3, method: 'turn/start',
    params: { input: [{ type: 'text', text: 'stream fixture' }] },
  });
}

async function completeInteractions(fixture) {
  const command = await fixture.waitFor(message => message.method === 'item/commandExecution/requestApproval');
  fixture.send({ jsonrpc: '2.0', id: command.id, result: { decision: 'accept' } });
  const file = await fixture.waitFor(message => message.method === 'item/fileChange/requestApproval');
  fixture.send({ jsonrpc: '2.0', id: file.id, result: { decision: 'accept' } });
  const input = await fixture.waitFor(message => message.method === 'item/tool/requestUserInput');
  fixture.send({ jsonrpc: '2.0', id: input.id, result: { answers: { confirm: { answers: ['yes'] } } } });
  return fixture.waitFor(message => message.method === 'turn/completed');
}

async function defaultFlowRetainsOneImmediateDelta() {
  const inheritedError = process.env.MONITTER_FIXTURE_ERROR;
  process.env.MONITTER_FIXTURE_ERROR = '1';
  const fixture = startFixture();
  if (inheritedError === undefined) delete process.env.MONITTER_FIXTURE_ERROR;
  else process.env.MONITTER_FIXTURE_ERROR = inheritedError;
  try {
    await startTurn(fixture);
    await completeInteractions(fixture);
    const deltas = fixture.all().filter(message => message.method === 'item/agentMessage/delta');
    assert.deepEqual(deltas.map(message => message.params.delta), ['fixture response']);
    assert.equal(deltas[0].params.threadId, '00000000-0000-7000-8000-000000000001');
    assert.equal(deltas[0].params.turnId, '00000000-0000-7000-8000-000000000002');
    const completed = fixture.all().find(message => message.method === 'item/completed' && message.params.item?.type === 'agentMessage');
    assert.equal(completed.params.item.text, 'fixture response');
  } finally {
    await fixture.stop();
  }
}

async function pacedFlowAccumulatesFiniteDeltas() {
  const fixture = startFixture({
    MONITTER_FIXTURE_DELTA_TICKS: '3',
    MONITTER_FIXTURE_DELTA_INTERVAL_MS: '2',
  });
  try {
    await startTurn(fixture);
    await completeInteractions(fixture);
    const deltas = fixture.all().filter(message => message.method === 'item/agentMessage/delta');
    assert.deepEqual(deltas.map(message => message.params.delta), [
      'fixture response', ' fixture response 2', ' fixture response 3',
    ]);
    const completed = fixture.all().find(message => message.method === 'item/completed' && message.params.item?.type === 'agentMessage');
    assert.equal(completed.params.item.text, 'fixture response fixture response 2 fixture response 3');
  } finally {
    await fixture.stop();
  }
}

const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));

async function interruptedPacedTurnCannotLeakIntoReplacement() {
  const fixture = startFixture({
    MONITTER_FIXTURE_DELTA_TICKS: '5',
    MONITTER_FIXTURE_DELTA_INTERVAL_MS: '60',
  });
  try {
    await startTurn(fixture);
    const firstDelta = await fixture.waitFor(message => message.method === 'item/agentMessage/delta');
    const interruptedTurnId = firstDelta.params.turnId;
    const boundary = fixture.all().length;
    fixture.send({
      jsonrpc: '2.0', id: 70, method: 'turn/interrupt',
      params: { threadId: firstDelta.params.threadId, turnId: interruptedTurnId },
    });
    await fixture.waitFor(message => message.id === 70 && message.result);
    await fixture.waitFor(message => message.method === 'turn/completed' && message.params.turn.id === interruptedTurnId);

    fixture.send({
      jsonrpc: '2.0', id: 4, method: 'turn/start',
      params: { input: [{ type: 'text', text: 'replacement fixture' }] },
    });
    await fixture.waitFor(message => message.id === 4 && message.result);
    const replacementDelta = await fixture.waitFor(message =>
      message.method === 'item/agentMessage/delta' && message.params.turnId !== interruptedTurnId,
    );
    await completeInteractions(fixture);
    await delay(350);

    const afterInterrupt = fixture.all().slice(boundary);
    assert.equal(
      afterInterrupt.filter(message => message.method === 'item/agentMessage/delta' && message.params.turnId === interruptedTurnId).length,
      0,
    );
    assert.equal(
      afterInterrupt.filter(message => message.method === 'item/commandExecution/requestApproval' && message.params.turnId === interruptedTurnId).length,
      0,
    );
    assert.equal(
      afterInterrupt.filter(message => message.method === 'item/completed' && message.params.turnId === interruptedTurnId).length,
      0,
    );
    assert.equal(replacementDelta.params.delta, 'fixture response');
  } finally {
    await fixture.stop();
  }
}

async function staleApprovalCannotResolveReplacementTurn() {
  const fixture = startFixture();
  try {
    await startTurn(fixture);
    const originalApproval = await fixture.waitFor(message => message.method === 'item/commandExecution/requestApproval');
    fixture.send({
      jsonrpc: '2.0', id: 4, method: 'turn/start',
      params: { input: [{ type: 'text', text: 'replacement while approval is pending' }] },
    });
    await fixture.waitFor(message => message.id === 4 && message.result);
    const replacementApproval = await fixture.waitFor(message =>
      message.method === 'item/commandExecution/requestApproval' && message.id !== originalApproval.id,
    );

    fixture.send({ jsonrpc: '2.0', id: originalApproval.id, result: { decision: 'accept' } });
    const staleError = await fixture.waitFor(message => message.id === originalApproval.id && message.error);
    assert.match(staleError.error.message, /does not match the pending request/);
    assert.equal(
      fixture.all().filter(message => message.method === 'item/fileChange/requestApproval').length,
      0,
    );

    fixture.send({ jsonrpc: '2.0', id: replacementApproval.id, result: { decision: 'accept' } });
    const file = await fixture.waitFor(message => message.method === 'item/fileChange/requestApproval');
    fixture.send({ jsonrpc: '2.0', id: file.id, result: { decision: 'accept' } });
    const input = await fixture.waitFor(message => message.method === 'item/tool/requestUserInput');
    fixture.send({ jsonrpc: '2.0', id: input.id, result: { answers: { confirm: { answers: ['yes'] } } } });
    await fixture.waitFor(message => message.method === 'turn/completed' && message.params.turn.id === replacementApproval.params.turnId);
  } finally {
    await fixture.stop();
  }
}

async function uniqueIdModeSeparatesFixtureChildren() {
  const first = startFixture({ MONITTER_FIXTURE_UNIQUE_IDS: '1' });
  const second = startFixture({ MONITTER_FIXTURE_UNIQUE_IDS: '1' });
  try {
    await Promise.all([startTurn(first), startTurn(second)]);
    const firstThread = first.all().find(message => message.id === 2 && message.result)?.result.thread.id;
    const secondThread = second.all().find(message => message.id === 2 && message.result)?.result.thread.id;
    assert.match(firstThread, /^[0-9a-f-]{36}$/);
    assert.match(secondThread, /^[0-9a-f-]{36}$/);
    assert.notEqual(firstThread, secondThread);
  } finally {
    await Promise.all([first.stop(), second.stop()]);
  }
}

async function uniqueIdResumeBindsSubsequentNotifications() {
  const requestedThreadId = '11111111-2222-4333-8444-555555555555';
  const fixture = startFixture({ MONITTER_FIXTURE_UNIQUE_IDS: '1' });
  const enabledTools = [
    'list_agents', 'delegate_task', 'send_message', 'get_task_result', 'wait_for_task',
    'list_messages', 'cancel_delegation', 'terminal_run', 'skills_help',
    'list_shared_skills', 'install_shared_skill',
  ];
  try {
    fixture.send({ jsonrpc: '2.0', id: 1, method: 'initialize', params: {} });
    await fixture.waitFor(message => message.id === 1 && message.result);
    fixture.send({ jsonrpc: '2.0', method: 'initialized', params: {} });
    fixture.send({
      jsonrpc: '2.0', id: 2, method: 'thread/resume',
      params: {
        threadId: requestedThreadId,
        excludeTurns: true,
        config: {
          'mcp_servers.monitter.required': true,
          'mcp_servers.monitter.url': 'http://127.0.0.1:4444/mcp',
          'mcp_servers.monitter.bearer_token_env_var': 'MONITTER_TOKEN',
          'mcp_servers.monitter.enabled_tools': enabledTools,
        },
      },
    });
    const resumed = await fixture.waitFor(message => message.id === 2 && message.result);
    assert.equal(resumed.result.thread.id, requestedThreadId);
    fixture.send({
      jsonrpc: '2.0', id: 3, method: 'turn/start',
      params: { input: [{ type: 'text', text: 'resumed fixture' }] },
    });
    const delta = await fixture.waitFor(message => message.method === 'item/agentMessage/delta');
    assert.equal(delta.params.threadId, requestedThreadId);
  } finally {
    await fixture.stop();
  }
}

async function invalidBoundsRejectBeforeStreaming() {
  for (const [environment, expected] of [
    [{ MONITTER_FIXTURE_DELTA_TICKS: '601' }, /1 through 600/],
    [{ MONITTER_FIXTURE_DELTA_TICKS: '600', MONITTER_FIXTURE_DELTA_INTERVAL_MS: '101' }, /60000 ms/],
    [{ MONITTER_FIXTURE_DELTA_TICKS: 'one' }, /integer from 1 through 600/],
    [{ MONITTER_FIXTURE_DELTA_INTERVAL_MS: '10' }, /requires MONITTER_FIXTURE_DELTA_TICKS/],
  ]) {
    const fixture = startFixture(environment);
    try {
      await startTurn(fixture);
      const error = await fixture.waitFor(message => message.id === 3 && message.error);
      assert.match(error.error.message, expected);
      assert.equal(fixture.all().filter(message => message.method === 'item/agentMessage/delta').length, 0);
      assert.equal(fixture.all().filter(message => message.method === 'item/commandExecution/requestApproval').length, 0);
    } finally {
      await fixture.stop();
    }
  }
}

await defaultFlowRetainsOneImmediateDelta();
await pacedFlowAccumulatesFiniteDeltas();
await interruptedPacedTurnCannotLeakIntoReplacement();
await staleApprovalCannotResolveReplacementTurn();
await uniqueIdModeSeparatesFixtureChildren();
await uniqueIdResumeBindsSubsequentNotifications();
await invalidBoundsRejectBeforeStreaming();
console.log('Codex app-server mock default, paced, and invalid streaming bounds passed.');
