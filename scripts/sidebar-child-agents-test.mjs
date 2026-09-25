import assert from 'node:assert/strict';
import { sidebarChildAgents, sidebarRootTasks } from '../src/lib/sidebar-child-agents.ts';

const task = (id, overrides = {}) => ({
  id, agentId: `agent-${id}`, title: `Task ${id}`, nativeSessionId: null,
  archived: false, status: 'running', createdAt: 1, updatedAt: 1,
  parentTaskId: null, channelId: null, projectId: null, hostId: 'host', cwd: '/',
  provider: 'codex', model: 'test', sandbox: 'read-only', ...overrides,
});
const agent = (id, name) => ({
  id, name, description: '', instructions: '', avatar: null, provider: 'codex', model: 'test',
  hostId: 'host', cwd: '/', color: '#000', sandbox: 'read-only', expertise: [],
  responsibilities: [], skills: [], collaborationEnabled: true,
});
const collaboration = (id, parent, child, overrides = {}) => ({
  id, kind: 'delegation', fromAgentId: `agent-${parent}`, fromTaskId: parent,
  toAgentId: `agent-${child}`, toTaskId: child, text: `Please handle ${child}`,
  requestId: `request-${id}`, status: 'running', result: null, error: null,
  createdAt: 2, updatedAt: 2, ...overrides,
});

const parent = task('parent');
const nativeChild = task('native-child', { agentId: 'codex-child', title: 'Codex child task', parentTaskId: null, updatedAt: 10 });
const acpChild = task('acp-child', { agentId: 'acp-child-agent', title: 'ACP child task', updatedAt: 11 });
const delegatedChild = task('delegated-child', { agentId: 'worker', title: 'Mapped collaboration task', updatedAt: 12 });
const directChild = task('direct-child', { title: 'Direct task child', parentTaskId: 'parent', updatedAt: 4 });
const completedChild = task('completed-child', { title: 'Completed direct task', parentTaskId: 'parent', status: 'completed', updatedAt: 99 });
const hiddenChild = task('hidden-child', { title: 'Child whose parent is hidden', parentTaskId: 'archived-parent' });

const snapshot = {
  agents: [agent('codex-child', 'Codex Worker'), agent('acp-child-agent', 'ACP Worker'), agent('worker', 'Mapped Worker')],
  collaborations: [
    collaboration('mapped', 'parent', 'delegated-child', { status: 'completed', updatedAt: 6 }),
    collaboration('fallback', 'parent', 'fallback-child', { status: 'queued', updatedAt: 3 }),
  ],
  subagentSessions: [
    { id: 'codex-session', source: 'codex', parentTaskId: 'parent', agentPath: 'agent/codex-worker', prompt: 'Codex native session', status: 'running', createdAt: 2, updatedAt: 5 },
    { id: 'acp-session', source: 'acp', parentTaskId: 'parent', agentPath: 'agent/acp-worker', prompt: 'ACP native session', status: 'completed', createdAt: 2, updatedAt: 7 },
    { id: 'mapped-session', source: 'collaboration', parentTaskId: 'parent', collaborationId: 'mapped', prompt: 'Mapped delegation session', status: 'running', createdAt: 2, updatedAt: 8 },
  ],
};
const visibleTasks = [parent, nativeChild, acpChild, delegatedChild, directChild, completedChild];
const rows = sidebarChildAgents(snapshot, visibleTasks).get('parent');

assert.ok(rows, 'visible parent has child rows');
assert.equal(rows.length, 6, 'session mapped to collaboration is emitted once; fallback and direct tasks are included');
assert.deepEqual(rows.map(row => row.id), [
  'mapped-session', 'codex-session', 'task:direct-child', 'collaboration:fallback', 'task:completed-child', 'acp-session',
]);
assert.equal(rows.find(row => row.id === 'mapped-session').task.id, 'delegated-child');
assert.equal(rows.filter(row => row.task?.id === 'delegated-child').length, 1, 'collaboration task is deduplicated against its session');
assert.equal(rows.find(row => row.id === 'collaboration:fallback').sessionId, 'collaboration:fallback', 'older collaboration projection has a stable fallback identifier');
assert.equal(rows.find(row => row.id === 'task:direct-child').sessionId, null, 'visible direct child task works without a session or collaboration');
assert.equal(rows.find(row => row.id === 'codex-session').agentName, 'Codex Worker');
assert.equal(rows.find(row => row.id === 'acp-session').agentName, 'Acp Worker');

const noParentRow = sidebarChildAgents(snapshot, [hiddenChild]);
assert.equal(noParentRow.size, 0, 'hidden or archived parent cannot receive a nested child row');
assert.deepEqual(sidebarRootTasks([hiddenChild]), [hiddenChild], 'child becomes a root row when its parent is not visible');
assert.deepEqual(sidebarRootTasks([parent, directChild]).map(item => item.id), ['parent'], 'visible child is not duplicated at the root');

const crossAgentParent = task('cross-agent-parent', { agentId: 'different-owner' });
const crossAgentChild = task('cross-agent-child', { agentId: 'separate-worker', parentTaskId: crossAgentParent.id });
const crossAgentRows = sidebarChildAgents({ agents: [], collaborations: [], subagentSessions: [] }, [crossAgentParent, crossAgentChild]);
assert.equal(crossAgentRows.get(crossAgentParent.id)?.[0]?.task?.id, crossAgentChild.id, 'parentTaskId controls nesting across agent ownership');

console.log('Sidebar child agent helpers cover native sessions, collaboration mapping and fallback, direct children, parent visibility, cross-agent nesting, deduplication, and status sorting.');
