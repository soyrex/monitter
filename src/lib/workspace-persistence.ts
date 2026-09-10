import type { PaneLayout } from '$lib/panes';

const key = 'monitter.workspace.v1';

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

export function loadWorkspace(): PersistedWorkspace | null {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(key) ?? 'null');
    if (!value || typeof value !== 'object') return null;
    const saved = value as Partial<PersistedWorkspace>;
    if (saved.version !== 1 || !isLayout(saved.layout) || paneIds(saved.layout).length > 4
      || !paneIds(saved.layout).includes('main') || new Set(paneIds(saved.layout)).size !== paneIds(saved.layout).length || typeof saved.activePaneId !== 'string'
      || !saved.main || typeof saved.main !== 'object' || !saved.panes || typeof saved.panes !== 'object'
      || !Array.isArray(saved.terminals)) return null;
    return clone(saved as PersistedWorkspace);
  } catch { return null; }
}

export function saveWorkspace(workspace: PersistedWorkspace): string | null {
  try { localStorage.setItem(key, JSON.stringify(workspace)); return null; }
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
