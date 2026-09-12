import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { collectWorkspaceSidebarTabs } from '../src/lib/workspace-sidebar-tabs.ts';

const terminal = (id, title) => ({ id, title, hostId: 'host', cwd: `/tmp/${id}`, status: 'running', exitCode: null });
const terminals = { one: terminal('one', 'Build shell'), two: terminal('two', 'Server logs') };

const panes = [
  { openTerminalIds: ['one', 'two'], openDraftIds: ['draft'], openEmptyIds: ['empty'], taskDrafts: { draft: { title: 'Plan release' } }, settingsOpen: true, tabOrder: [{ kind: 'settings', id: 'settings' }, { kind: 'terminal', id: 'two' }, { kind: 'draft', id: 'draft' }, { kind: 'empty', id: 'empty' }, { kind: 'terminal', id: 'one' }] },
  { openTerminalIds: ['two'], openDraftIds: ['draft'], openEmptyIds: ['empty'], settingsOpen: true, tabOrder: [{ kind: 'terminal', id: 'two' }, { kind: 'settings', id: 'settings' }] },
];
const result = collectWorkspaceSidebarTabs(panes, terminals);
assert.deepEqual(result, [
  { kind: 'settings', id: 'settings', title: 'Settings' },
  { kind: 'terminal', id: 'two', title: 'Server logs' },
  { kind: 'draft', id: 'draft', title: 'Plan release' },
  { kind: 'empty', id: 'empty', title: 'New tab' },
  { kind: 'terminal', id: 'one', title: 'Build shell' },
]);

assert.deepEqual(
  collectWorkspaceSidebarTabs([{ openTerminalIds: ['one'], tabOrder: [{ kind: 'terminal', id: 'closed' }, { kind: 'settings', id: 'settings' }] }], terminals),
  [{ kind: 'terminal', id: 'one', title: 'Build shell' }],
  'stale tab order must not resurrect a closed terminal or Settings tab',
);

assert.deepEqual(
  collectWorkspaceSidebarTabs([{ openTerminalIds: ['gone'], tabOrder: [{ kind: 'terminal', id: 'gone' }] }], terminals),
  [{ kind: 'terminal', id: 'gone', title: 'Terminal unavailable', disabled: true }],
  'a missing live terminal stays non-clickable rather than opening a new terminal',
);

const renamed = collectWorkspaceSidebarTabs([{ openTerminalIds: ['one'] }], { one: terminal('one', 'Renamed terminal') });
assert.equal(renamed[0].title, 'Renamed terminal', 'live terminal metadata supplies the current title');

const input = [{ openTerminalIds: ['one'], settingsOpen: false, openEmptyIds: ['empty'], openDraftIds: ['draft'], taskDrafts: { draft: {} }, tabOrder: [{ kind: 'empty', id: 'empty' }, { kind: 'draft', id: 'draft' }, { kind: 'terminal', id: 'one' }, { kind: 'draft', id: 'closed' }] }];
const before = structuredClone(input);
assert.deepEqual(collectWorkspaceSidebarTabs(input, terminals), [
  { kind: 'empty', id: 'empty', title: 'New tab' },
  { kind: 'draft', id: 'draft', title: 'New chat' },
  { kind: 'terminal', id: 'one', title: 'Build shell' },
]);
assert.deepEqual(input, before, 'collector must not mutate persisted pane state');
assert.deepEqual(collectWorkspaceSidebarTabs([], terminals), []);
console.log('workspace sidebar tab collector assertions passed');

// Exercise the real sidebar routing functions directly from AppSurface. This
// keeps the test small without booting the Svelte application or copying its
// focus algorithm into a fixture.
const appSurface = readFileSync(new URL('../src/lib/components/AppSurface.svelte', import.meta.url), 'utf8');
const block = (startMarker, endMarker) => {
  const start = appSurface.indexOf(startMarker);
  const end = appSurface.indexOf(endMarker, start);
  assert.notEqual(start, -1, `missing AppSurface marker: ${startMarker}`);
  assert.notEqual(end, -1, `missing AppSurface boundary: ${endMarker}`);
  return appSurface.slice(start, end);
};
const focusSource = block('  export function focusExistingTab(tab: TabKey): boolean {', '  const sidebarWorkspaceTabs');
const openSource = block('  async function openAgentWorkspaceTab(agentId: string, tab: SidebarWorkspaceTab) {', '  const sidebarOpenTaskIds');
const routeSource = ts.transpileModule(`${focusSource.replace('export function', 'function')}\n${openSource}`, {
  compilerOptions: { target: ts.ScriptTarget.ESNext, module: ts.ModuleKind.ESNext },
}).outputText;
const routes = new Function('ctx', `with (ctx) { ${routeSource}; return { focusExistingTab, openAgentWorkspaceTab }; }`);

function createContext({ activeWorkspaceKey = 'agent:alex', owners = {}, switchResult = true, expandedPaneId = null } = {}) {
  const calls = [];
  const context = {
    activeWorkspaceKey,
    layout: { owners },
    paneRefs: {},
    expandedPaneId,
    collapsedAgents: {},
    mobileMain: false,
    notice: '',
    orderedTabs: () => owners.main ?? [],
    selectVimTab: target => { calls.push(['main-focus', target]); return true; },
    paneIds: layout => ['main', ...Object.keys(layout.owners).filter(id => id !== 'main')],
    allTabs: () => owners.main ?? [],
    setPaneExpansion: value => { calls.push(['expand', value]); context.expandedPaneId = value; },
    switchWorkspace: async scope => {
      calls.push(['switch', scope]);
      if (switchResult) context.activeWorkspaceKey = scope;
      return switchResult;
    },
  };
  for (const [id, tabs] of Object.entries(owners)) if (id !== 'main') {
    context.paneRefs[id] = {
      allTabs: () => tabs,
      focusExistingTab: tab => { calls.push([`${id}-focus`, tab]); return true; },
    };
  }
  return { context, calls, route: routes(context).openAgentWorkspaceTab };
}

{
  const { context, calls, route } = createContext({ activeWorkspaceKey: 'agent:other', owners: { main: [{ kind: 'terminal', id: 'one' }] } });
  await route('alex', { kind: 'terminal', id: 'one', title: 'Build shell' });
  assert.deepEqual(calls, [['switch', 'agent:alex'], ['main-focus', { kind: 'index', index: 1 }]]);
  assert.equal(context.activeWorkspaceKey, 'agent:alex');
  assert.equal(context.mobileMain, true);
  assert.equal(context.collapsedAgents.alex, false);
}

{
  const { context, calls, route } = createContext({ owners: { main: [], side: [{ kind: 'draft', id: 'draft' }] }, expandedPaneId: 'main' });
  await route('alex', { kind: 'draft', id: 'draft', title: 'Plan release' });
  assert.deepEqual(calls, [['switch', 'agent:alex'], ['expand', null], ['side-focus', { kind: 'draft', id: 'draft', title: 'Plan release' }]]);
  assert.equal(context.expandedPaneId, null, 'focusing another pane restores the normal layout');
}

{
  const { context, calls, route } = createContext({ owners: { main: [] } });
  await route('alex', { kind: 'empty', id: 'closed', title: 'New tab' });
  assert.deepEqual(calls, [['switch', 'agent:alex']]);
  assert.equal(context.notice, 'That tab is no longer open in this workspace.');
}

{
  const { calls, route } = createContext({ activeWorkspaceKey: 'agent:other', owners: { main: [{ kind: 'terminal', id: 'gone' }] } });
  await route('alex', { kind: 'terminal', id: 'gone', title: 'Terminal unavailable', disabled: true });
  assert.deepEqual(calls, [], 'disabled unavailable terminals must not route or recreate a session');
}

{
  const { context, calls, route } = createContext({ activeWorkspaceKey: 'agent:other', owners: { main: [{ kind: 'settings', id: 'settings' }] }, switchResult: false });
  await route('alex', { kind: 'settings', id: 'settings', title: 'Settings' });
  assert.deepEqual(calls, [['switch', 'agent:alex']]);
  assert.equal(context.activeWorkspaceKey, 'agent:other');
}
console.log('workspace sidebar routing assertions passed');
