#!/usr/bin/env node
// Focused comparison for AppSurface's immutable bridge Snapshot. This uses
// Svelte's own compiler and client proxy implementation; it is not a hand-
// rolled Proxy model. It intentionally benchmarks replacement + indexing,
// which is the production update shape (snapshots are never edited in place).
import { compile } from 'svelte/compiler';
import { proxy } from 'svelte/internal/client';

const deepSource = `<script>let snapshot = $state(input);</script><p>{snapshot.messages.length}</p>`;
const rawSource = `<script>let snapshot = $state.raw(input);</script><p>{snapshot.messages.length}</p>`;
const deepCompiled = compile(deepSource, { generate: 'client', hydratable: false, runes: true }).js.code;
const rawCompiled = compile(rawSource, { generate: 'client', hydratable: false, runes: true }).js.code;
if (!deepCompiled.includes('$.proxy') || rawCompiled.includes('$.proxy')) {
  throw new Error('Svelte compiler did not emit the expected deep/raw state forms.');
}

const TASKS = 1000;
const MESSAGES = 4000;
const ROUNDS = 25;
const snapshots = Array.from({ length: ROUNDS }, (_, round) => ({
  settings: { theme: 'dark', accent: '#3f9d6a', interfaceScale: 125 },
  agents: Array.from({ length: 40 }, (_, id) => ({ id: `agent-${id}`, name: `Agent ${id}` })),
  hosts: [{ id: 'local', name: 'Local' }],
  projects: [], channels: [], events: [], queuedMessages: [], approvalRequests: [], approvalRules: [], collaborations: [],
  tasks: Array.from({ length: TASKS }, (_, id) => ({ id: `task-${id}`, agentId: `agent-${id % 40}`, archived: id % 11 === 0, status: id % 17 === 0 ? 'running' : 'completed', updatedAt: id + round })),
  messages: Array.from({ length: MESSAGES }, (_, id) => ({ id: `message-${id}`, taskId: `task-${id % TASKS}`, role: id % 2 ? 'assistant' : 'user', text: `message ${id}`, createdAt: id + round })),
}));

function indexAndSelect(snapshot) {
  const taskById = new Map(snapshot.tasks.map((task) => [task.id, task]));
  const messagesByTask = new Map();
  for (const message of snapshot.messages) {
    const values = messagesByTask.get(message.taskId);
    if (values) values.push(message); else messagesByTask.set(message.taskId, [message]);
  }
  const selected = taskById.get('task-777');
  const selectedMessages = messagesByTask.get(selected.id) ?? [];
  return selectedMessages.length + (selected.archived ? 1 : 0);
}

function bench(label, prepare) {
  // Warm the Svelte proxy traps and JIT before measuring.
  for (let i = 0; i < 3; i++) for (const snapshot of snapshots) indexAndSelect(prepare(snapshot));
  const start = process.hrtime.bigint();
  let checksum = 0;
  for (let i = 0; i < 4; i++) for (const snapshot of snapshots) checksum += indexAndSelect(prepare(snapshot));
  const ms = Number(process.hrtime.bigint() - start) / 1e6;
  return { label, ms, checksum };
}

const deep = bench('deep $state (Svelte proxy)', (snapshot) => proxy(snapshot));
const raw = bench('$state.raw (plain immutable object)', (snapshot) => snapshot);
console.log(JSON.stringify({
  svelte: process.env.npm_package_version ?? 'installed',
  workload: { tasks: TASKS, messages: MESSAGES, snapshots: ROUNDS, measuredPasses: 4 },
  compiler: { deepEmitsProxy: deepCompiled.includes('$.proxy'), rawEmitsProxy: rawCompiled.includes('$.proxy') },
  results: { deep, raw, deepOverRaw: deep.ms / raw.ms, rawImprovementPercent: (1 - raw.ms / deep.ms) * 100 },
}, null, 2));
