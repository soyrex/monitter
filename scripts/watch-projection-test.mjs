import { strict as assert } from 'node:assert';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
const modules = new Map();
function load(name) {
  if (modules.has(name)) return modules.get(name);
  const source = readFileSync(new URL(`../src/lib/${name}.ts`, import.meta.url), 'utf8');
  const exports = {};
  const js = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText;
  runInNewContext(js, { exports, require: dependency => {
    assert.ok(dependency.startsWith('$lib/'));
    return load(dependency.slice(5));
  }});
  modules.set(name, exports);
  return exports;
}
const { projectWatch, selectWatchBuild, selectWatchTeam, watchGoal } = load('watch');
const task = { id: 'owner', projectId: 'project', status: 'running' };
const snapshot = { tasks: [task], agents: [], projects: [], collaborations: [], subagentSessions: [], messages: [], events: [], workPlans: [], buildRequests: [], buildRuns: [] };
const request = (id, status = 'ready', extra = {}) => ({ id, taskId: task.id, projectId: task.projectId, status, title: id, createdAt: 50, updatedAt: 50, ...extra });
const run = (extra = {}) => ({ id: 'run', status: 'running', projectId: task.projectId, ownerTaskId: 'peer', requestIds: ['included'], leaseExpiresAt: 200, createdAt: 50, ...extra });
snapshot.buildRequests = [request('included'), request('foreign', 'ready', { projectId: 'elsewhere' })];
snapshot.buildRuns = [run()];
assert.equal(selectWatchBuild(snapshot, task, 100).label, 'Building');
assert.equal(selectWatchBuild(snapshot, task, 201).label, 'Build needs attention');
assert.equal(selectWatchBuild(snapshot, task, 200).stale, false);
snapshot.buildRuns = [run({ projectId: 'elsewhere' })];
assert.equal(selectWatchBuild(snapshot, task, 100).label, 'Waiting for build claim');
snapshot.buildRuns = [run({ ownerTaskId: task.id, requestIds: [] })];
assert.equal(selectWatchBuild(snapshot, task, 100).title, 'Current build');
snapshot.buildRuns = [];
snapshot.buildRequests = [request('planned', 'planned', { createdAt: 90 }), request('ready')];
assert.equal(selectWatchBuild(snapshot, task, 100).title, 'ready');
snapshot.buildRequests = [request('planned', 'planned')];
assert.equal(selectWatchBuild(snapshot, task, 100).label, 'Preparing build');
assert.equal(selectWatchBuild(snapshot, { ...task, projectId: null }, 100), null);
snapshot.messages = [
  { id: 'old', taskId: task.id, role: 'user', text: 'old', createdAt: 10 },
  { id: 'request', taskId: task.id, role: 'user', text: 'Current request', createdAt: 20 },
  { id: 'peer', taskId: task.id, role: 'user', text: 'Peer traffic', senderAgentId: 'peer', createdAt: 30 },
  { id: 'elsewhere', taskId: 'other', role: 'user', text: 'other', createdAt: 99 },
];
snapshot.events = [
  { id: 'old-progress', taskId: task.id, kind: 'agent_progress', title: 'Old', createdAt: 25 },
  { id: 'progress', taskId: task.id, kind: 'agent_progress', title: 'Current', createdAt: 31 },
  { id: 'tool', taskId: task.id, kind: 'computer', title: 'Inspecting', createdAt: 32 },
  { id: 'foreign', taskId: 'other', kind: 'agent_progress', title: 'Other', createdAt: 99 },
];
const plan = (id, status, updatedAt) => ({ id, taskId: task.id, status, createdAt: 15, updatedAt, items: [] });
snapshot.workPlans = [plan('active', 'active', 25), plan('closed', 'closed', 40)];
let view = projectWatch(snapshot, task, 100);
assert.equal(view.request.id, 'request');
assert.equal(view.activePlan.id, 'active');
assert.equal(view.finishedPlan.id, 'closed');
assert.equal(view.progress.id, 'progress');
assert.equal(view.tool.id, 'tool');
assert.equal(view.activity.length, 2);
snapshot.workPlans = [plan('closed', 'closed', 40)];
assert.equal(projectWatch(snapshot, task).activePlan, null);
snapshot.messages = [];
view = projectWatch(snapshot, task);
assert.equal(view.progress, null);
assert.equal(view.activity.length, 0);
const team = [
  { id: 'old-active', status: 'running', launchedAt: 1, updatedAt: 2 },
  { id: 'old-done', status: 'completed', launchedAt: 1, updatedAt: 2 },
  { id: 'recent-done', status: 'completed', launchedAt: 20, updatedAt: 30 },
  { id: 'queued', status: 'queued', launchedAt: 10, updatedAt: 10 },
];
assert.equal(selectWatchTeam(team, 20).map(row => row.id).join(','), 'old-active,queued,recent-done');
const goalPlan = { items: [{ status: 'completed' }, { status: 'skipped' }, { status: 'blocked', title: 'Waiting for review' }, { status: 'in_progress', title: 'Verifying current work' }] };
const goalView = watchGoal({ status: 'active', tokensUsed: 0, tokenBudget: 1000, timeUsedSeconds: 65 }, goalPlan, { title: 'Latest update' });
assert.equal(goalView.completed, 1);
assert.equal(goalView.total, 4);
assert.equal(goalView.current, 'Verifying current work');
assert.equal(goalView.usage, '0 / 1,000 tokens used · 1:05 elapsed');
assert.equal(watchGoal({ status: 'blocked' }, { items: [{ status: 'blocked', title: 'Review' }] }, null).current, 'Review');
assert.equal(watchGoal({ status: 'unexpected', tokensUsed: NaN, tokenBudget: 0, timeUsedSeconds: -1 }, null, null).usage, '');
assert.equal(watchGoal({ status: 'unexpected' }, null, null).label, 'Goal status unknown');
assert.equal(watchGoal({ status: 'complete' }, null, { title: 'Finished' }).current, 'Finished');
console.log('Watch projection checks passed');
