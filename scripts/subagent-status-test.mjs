import { strict as assert } from 'node:assert';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';

const modules = new Map();
function load(name) {
  if (modules.has(name)) return modules.get(name);
  const source = readFileSync(new URL(`../src/lib/${name}.ts`, import.meta.url), 'utf8');
  const js = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const exports = {};
  runInNewContext(js, { exports, require: dependency => {
    assert.equal(dependency, '$lib/activity-grouping');
    return load('activity-grouping');
  }});
  modules.set(name, exports);
  return exports;
}

const { unifiedSubagentsForTask, activeUnifiedSubagent } = load('unified-subagents');
const input = {
  parentTaskId: 'parent', tasks: [], agents: [], collaborations: [],
  events: [{ id: 'late', taskId: 'parent', kind: 'subagent', title: 'Sub-agent activity', createdAt: 9000,
    detail: JSON.stringify({ type: 'subAgentActivity', kind: 'interacted', agentThreadId: 'child' }) }],
  subagentSessions: [{ id: 'codex:child', source: 'codex', parentTaskId: 'parent',
    agentThreadId: 'child', agentPath: '/root/codex_steering', status: 'running',
    createdAt: 100, updatedAt: 200 }],
};
assert.equal(unifiedSubagentsForTask(input)[0].updatedAt, 9000);
assert.equal(activeUnifiedSubagent(unifiedSubagentsForTask(input)[0]), true);
for (const source of ['codex', 'acp']) {
  input.subagentSessions[0].source = source;
  for (const status of ['completed', 'error', 'interrupted']) {
    input.subagentSessions[0].status = status;
    const [row] = unifiedSubagentsForTask(input);
    assert.equal(row.status, status);
    assert.equal(activeUnifiedSubagent(row), false);
    assert.equal(row.updatedAt, 200);
    assert.equal(row.launchedAt, 100);
  }
}
const rootInput = {
  parentTaskId: 'parent', tasks: [{ id: 'parent', nativeSessionId: 'parent-thread' }, { id: 'other-parent', nativeSessionId: 'child-thread' }],
  agents: [], collaborations: [], subagentSessions: [
    { id: 'self', source: 'codex', parentTaskId: 'parent', agentThreadId: 'parent-thread', agentPath: '/root', status: 'running', createdAt: 100, updatedAt: 200 },
    { id: 'child', source: 'codex', parentTaskId: 'parent', agentThreadId: 'child-thread', agentPath: '/root', status: 'running', createdAt: 100, updatedAt: 200 },
    { id: 'acp', source: 'acp', parentTaskId: 'parent', agentThreadId: 'parent-thread', status: 'running', createdAt: 100, updatedAt: 200 },
    { id: 'delegated', source: 'collaboration', parentTaskId: 'parent', agentThreadId: 'parent-thread', status: 'completed', createdAt: 100, updatedAt: 200 },
    { id: 'unknown-thread', source: 'codex', parentTaskId: 'parent', agentThreadId: '', status: 'completed', createdAt: 100, updatedAt: 200 },
  ],
};
const rootIds = Array.from(unifiedSubagentsForTask(rootInput), row => row.id).sort();
assert.deepEqual(rootIds, ['acp', 'child', 'delegated', 'unknown-thread']);
for (const nativeSessionId of [null, '']) {
  rootInput.tasks[0].nativeSessionId = nativeSessionId;
  assert.equal(unifiedSubagentsForTask(rootInput).length, 5, 'missing/empty parent identity cannot establish a self row');
}
console.log('Native terminal subagents leave Active, retain actual end times, and exclude exact Codex parent roots.');
