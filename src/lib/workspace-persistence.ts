import type { PaneLayout } from '$lib/panes';
import type { Task } from '$lib/types';

const key = 'monitter.workspace.v1';
const workspaceSetKey = 'monitter.workspaces.v2';

export type WorkspaceKey = 'all' | `agent:${string}` | `project:${string}`;
export type PersistedWorkspaceSet = {
  version: 2;
  activeWorkspaceKey: WorkspaceKey;
  workspaces: Record<string, PersistedWorkspace>;
};

export function isWorkspaceKey(value: unknown): value is WorkspaceKey {
  return typeof value === 'string' && (value === 'all' || /^(agent|project):.+$/.test(value));
}

export function taskBelongsToWorkspace(task: Pick<Task, 'agentId' | 'projectId'>, scope: WorkspaceKey): boolean {
  if (scope === 'all') return true;
  if (scope.startsWith('agent:')) return task.agentId === scope.slice(6);
  return (task.projectId || 'unassigned') === scope.slice(8);
}

/** Navigate within the current grouping, retaining a compatible scope. */
export function workspaceForTask(task: Pick<Task, 'agentId' | 'projectId'>, preferred: WorkspaceKey): WorkspaceKey {
  if (taskBelongsToWorkspace(task, preferred)) return preferred;
  if (preferred.startsWith('agent:')) return `agent:${task.agentId}`;
  if (preferred.startsWith('project:')) return `project:${task.projectId || 'unassigned'}`;
  return 'all';
}

export type PersistedTerminal = { id: string; hostId: string; cwd: string };
export type PersistedWorkspace = {
  version: 1;
  layout: PaneLayout;
  activePaneId: string;
  main: Record<string, unknown>;
  panes: Record<string, Record<string, unknown>>;
  sidebarCollapsed: boolean;
  collapsedAgents: Record<string, boolean>;
  collapsedProjects: Record<string, boolean>;
  terminals: PersistedTerminal[];
};

function clone<T>(value: T): T { return JSON.parse(JSON.stringify(value)); }

function isLayout(value: unknown): value is PaneLayout {
  if (!value || typeof value !== 'object') return false;
  const node = value as Record<string, unknown>;
  if (typeof node.id !== 'string') return false;
  if (!('axis' in node)) return Object.keys(node).every(key => key === 'id');
  return (node.axis === 'horizontal' || node.axis === 'vertical')
    && typeof node.ratio === 'number' && Number.isFinite(node.ratio) && node.ratio >= .15 && node.ratio <= .85
    && isLayout(node.first) && isLayout(node.second);
}
function paneIds(layout: PaneLayout): string[] { return 'axis' in layout ? [...paneIds(layout.first), ...paneIds(layout.second)] : [layout.id]; }

function isWorkspace(value: unknown): value is PersistedWorkspace {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const saved = value as Partial<PersistedWorkspace>;
  if (saved.version !== 1 || !isLayout(saved.layout) || paneIds(saved.layout).length > 4
    || !paneIds(saved.layout).includes('main') || new Set(paneIds(saved.layout)).size !== paneIds(saved.layout).length
    || typeof saved.activePaneId !== 'string' || !saved.main || typeof saved.main !== 'object' || Array.isArray(saved.main)
    || !saved.panes || typeof saved.panes !== 'object' || Array.isArray(saved.panes) || !Array.isArray(saved.terminals)) return false;
  return Object.values(saved.panes).every(pane => !!pane && typeof pane === 'object' && !Array.isArray(pane))
    && saved.terminals.every(terminal => !!terminal && typeof terminal === 'object'
      && typeof terminal.id === 'string' && typeof terminal.hostId === 'string' && typeof terminal.cwd === 'string');
}

export function loadWorkspace(): PersistedWorkspace | null {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(key) ?? 'null');
    return isWorkspace(value) ? clone(value) : null;
  } catch { return null; }
}

export function saveWorkspace(workspace: PersistedWorkspace): string | null {
  try { localStorage.setItem(key, JSON.stringify(workspace)); return null; }
  catch (reason) { return reason instanceof Error ? reason.message : String(reason); }
}

/** Keep the legacy record intact so migration never destroys the old tab set. */
export function loadWorkspaceSet(): PersistedWorkspaceSet | null {
  const raw = localStorage.getItem(workspaceSetKey);
  if (raw === null) {
    const legacyRaw = localStorage.getItem(key);
    const legacy = loadWorkspace();
    if (legacyRaw !== null && !legacy) throw new Error('The previous workspace could not be read. Its stored data has been preserved.');
    return legacy ? { version: 2, activeWorkspaceKey: 'all', workspaces: { all: legacy } } : null;
  }
  let value: unknown;
  try { value = JSON.parse(raw); }
  catch { throw new Error('Saved workspaces could not be read. Their stored data has been preserved.'); }
  if (!isWorkspaceSet(value)) throw new Error('Saved workspaces have an unsupported or invalid format. Their stored data has been preserved.');
  return clone(value);
}

function isWorkspaceSet(value: unknown): value is PersistedWorkspaceSet {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const set = value as Partial<PersistedWorkspaceSet>;
  return set.version === 2 && isWorkspaceKey(set.activeWorkspaceKey)
    && !!set.workspaces && typeof set.workspaces === 'object' && !Array.isArray(set.workspaces)
    && Object.hasOwn(set.workspaces, set.activeWorkspaceKey)
    && Object.entries(set.workspaces).every(([scope, workspace]) => isWorkspaceKey(scope) && isWorkspace(workspace));
}

export function saveWorkspaceSet(set: PersistedWorkspaceSet): string | null {
  if (!isWorkspaceSet(set)) return 'Workspace state is invalid; existing saved workspaces were preserved.';
  try { localStorage.setItem(workspaceSetKey, JSON.stringify(set)); return null; }
  catch (reason) { return reason instanceof Error ? reason.message : String(reason); }
}

export function remapTerminalIds<T extends Record<string, unknown>>(state: T, ids: Record<string, string>): T {
  const result = clone(state) as Record<string, unknown>;
  const tabs = Array.isArray(result.openTerminalIds) ? result.openTerminalIds : [];
  result.openTerminalIds = tabs.flatMap(id => typeof id === 'string' && ids[id] ? [ids[id]] : []);
  if (Array.isArray(result.tabOrder)) result.tabOrder = result.tabOrder.flatMap(tab => {
    if (!tab || typeof tab !== 'object') return [];
    const value = tab as { kind?: unknown; id?: unknown };
    if (value.kind !== 'terminal' || typeof value.id !== 'string') return [tab];
    return ids[value.id] ? [{ ...value, id: ids[value.id] }] : [];
  });
  result.selectedTerminalId = typeof result.selectedTerminalId === 'string' ? ids[result.selectedTerminalId] ?? null : null;
  return result as T;
}
