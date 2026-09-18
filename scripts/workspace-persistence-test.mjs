import assert from 'node:assert/strict';
import { loadWorkspaceSet, saveWorkspaceSet, remapTerminalIds, taskBelongsToWorkspace, workspaceForTask } from '../src/lib/workspace-persistence.ts';
import { MAX_WORKSPACE_PANES } from '../src/lib/panes.ts';

const data = new Map();
globalThis.localStorage = {
  getItem: key => data.get(key) ?? null,
  setItem: (key, value) => data.set(key, value),
};
const legacyKey = 'monitter.workspace.v1', setKey = 'monitter.workspaces.v2';
const pane = {
  openTaskIds: ['shared-chat'], selectedTaskId: 'shared-chat',
  drafts: { 'task:shared-chat': 'Unsent text with an attachment' },
  queuedAttachments: { 'task:shared-chat': [{ id: 'attachment-1', path: '/tmp/test.txt' }] },
  openDraftIds: [], taskDrafts: { closed: { id: 'closed', text: 'Keep this closed draft' } },
  openTerminalIds: ['shell-1'], selectedTerminalId: 'shell-1',
  tabOrder: [{ kind: 'task', id: 'shared-chat' }, { kind: 'terminal', id: 'shell-1' }],
};
const workspace = {
  version: 1, layout: { id: 'split-1', axis: 'horizontal', ratio: .65, first: { id: 'main' }, second: { id: 'right' } },
  activePaneId: 'right', main: pane, panes: { right: { ...pane, selectedTaskId: null } },
  sidebarCollapsed: true, collapsedAgents: { atlas: true }, collapsedProjects: {},
  terminals: [{ id: 'shell-1', hostId: 'local', cwd: '/tmp' }],
};

assert.equal(loadWorkspaceSet(), null);
data.set(legacyKey, '{broken legacy');
assert.throws(loadWorkspaceSet, /previous workspace.*preserved/);
assert.equal(data.has(setKey), false);
const legacy = JSON.stringify(workspace);
data.set(legacyKey, legacy);
const migrated = loadWorkspaceSet();
assert.equal(migrated.activeWorkspaceKey, 'all');
assert.deepEqual(migrated.workspaces.all, workspace);
assert.equal(data.get(legacyKey), legacy);
assert.equal(data.has(setKey), false);

const scoped = structuredClone(migrated);
scoped.activeWorkspaceKey = 'agent:atlas';
scoped.workspaces['agent:atlas'] = structuredClone(workspace);
scoped.workspaces['project:monitter'] = structuredClone(workspace);
assert.equal(saveWorkspaceSet(scoped), null);
assert.deepEqual(loadWorkspaceSet(), scoped);
assert.equal(data.get(legacyKey), legacy, 'legacy recovery copy stays intact');
scoped.workspaces['agent:atlas'].main.drafts['task:shared-chat'] = 'changed after saving';
assert.equal(loadWorkspaceSet().workspaces['agent:atlas'].main.drafts['task:shared-chat'], pane.drafts['task:shared-chat']);

const withPaneCount = count => {
  const result = structuredClone(scoped);
  const all = result.workspaces.all;
  all.layout = { id: 'main' };
  all.panes = {};
  for (let index = 1; index < count; index++) {
    const id = `pane-${index}`;
    all.layout = { id: `split-${index}`, axis: 'horizontal', ratio: .5, first: all.layout, second: { id } };
    all.panes[id] = structuredClone(pane);
  }
  all.activePaneId = `pane-${count - 1}`;
  return result;
};
const eightPanes = withPaneCount(MAX_WORKSPACE_PANES);
assert.equal(saveWorkspaceSet(eightPanes), null);
assert.deepEqual(loadWorkspaceSet(), eightPanes, 'eight panes survive workspace reload');
const eightPaneSave = data.get(setKey);
assert.match(saveWorkspaceSet(withPaneCount(MAX_WORKSPACE_PANES + 1)), /invalid/);
assert.equal(data.get(setKey), eightPaneSave, 'a ninth pane cannot replace the saved workspace');

const valid = data.get(setKey);
assert.match(saveWorkspaceSet({ ...scoped, activeWorkspaceKey: 'agent:missing' }), /invalid/);
assert.equal(data.get(setKey), valid, 'invalid save never overwrites saved workspaces');
const malformedTerminal = structuredClone(scoped);
malformedTerminal.workspaces.all.terminals = [null];
assert.match(saveWorkspaceSet(malformedTerminal), /invalid/);
assert.equal(data.get(setKey), valid);
const malformedPane = structuredClone(scoped);
malformedPane.workspaces.all.panes.right = null;
assert.match(saveWorkspaceSet(malformedPane), /invalid/);
assert.equal(data.get(setKey), valid);
data.set(setKey, '{broken');
assert.throws(loadWorkspaceSet, /preserved/);
assert.equal(data.get(setKey), '{broken');
data.set(setKey, JSON.stringify({ ...scoped, version: 99 }));
assert.throws(loadWorkspaceSet, /unsupported/);
data.set(setKey, valid);
const setItem = localStorage.setItem;
localStorage.setItem = () => { throw new Error('Quota exceeded'); };
assert.equal(saveWorkspaceSet(scoped), 'Quota exceeded');
assert.equal(data.get(setKey), valid);
localStorage.setItem = setItem;

const task = { agentId: 'atlas', projectId: 'monitter' };
assert(taskBelongsToWorkspace(task, 'all'));
assert(taskBelongsToWorkspace(task, 'agent:atlas'));
assert(taskBelongsToWorkspace(task, 'project:monitter'));
assert(!taskBelongsToWorkspace(task, 'agent:scout'));
assert(!taskBelongsToWorkspace(task, 'project:elsewhere'));
assert.equal(workspaceForTask(task, 'agent:scout'), 'agent:atlas');
assert.equal(workspaceForTask(task, 'project:elsewhere'), 'project:monitter');
assert.equal(workspaceForTask(task, 'all'), 'all');
assert.equal(workspaceForTask({ ...task, projectId: null }, 'project:monitter'), 'project:unassigned');

const remapped = remapTerminalIds(pane, { 'shell-1': 'shell-restored' });
assert.equal(remapped.selectedTerminalId, 'shell-restored');
assert.deepEqual(remapped.openTerminalIds, ['shell-restored']);
assert.equal(remapped.tabOrder[1].id, 'shell-restored');
assert.deepEqual(remapped.drafts, pane.drafts);
assert.deepEqual(remapped.queuedAttachments, pane.queuedAttachments);
assert.equal(pane.selectedTerminalId, 'shell-1');
console.log('Workspace persistence: migration, multiple scopes, recovery, quota errors, routing and terminal remapping passed.');
