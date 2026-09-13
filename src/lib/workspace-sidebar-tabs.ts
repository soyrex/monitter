import type { Host, TerminalSession } from '$lib/types';

type WorkspacePaneTabs = {
  openTerminalIds?: unknown;
  settingsOpen?: unknown;
  settingsCategory?: unknown;
  openEmptyIds?: unknown;
  openDraftIds?: unknown;
  taskDrafts?: unknown;
  tabOrder?: unknown;
};

export type SidebarWorkspaceTab = {
  kind: 'terminal' | 'settings' | 'empty' | 'draft';
  id: string;
  title: string;
  disabled?: boolean;
};

type TabOrderItem = { kind?: unknown; id?: unknown };

export function settingsTabTitle(category: unknown): string {
  const labels: Record<string, string> = {
    profile: 'Profile',
    appearance: 'Appearance', typography: 'Typography',
    behaviour: 'Permissions & behaviour', conversation: 'Conversation',
    agents: 'Agents', directory: 'Agent directory', lan: 'LAN access', remote: 'Remote control',
  };
  const label = typeof category === 'string' && Object.hasOwn(labels, category)
    ? labels[category] : labels.appearance;
  return `Setting: ${label}`;
}

/** Give a new remote shell useful context until it receives a custom/automatic title. */
export function terminalTabTitle(session: TerminalSession, hosts: readonly Host[] = []): string {
  if (session.title !== 'Terminal') return session.title;
  const host = hosts.find(item => item.id === session.hostId);
  return host?.kind === 'ssh' ? `SSH: ${host.name.trim() || host.address.trim() || 'remote'}` : session.title;
}

/**
 * Lists the non-chat tabs belonging to one already-scoped workspace. Terminal
 * membership comes from each pane's open IDs, never from host or cwd metadata.
 * The stored tab order is advisory only: stale closed entries are ignored.
 */
export function collectWorkspaceSidebarTabs(
  panes: readonly WorkspacePaneTabs[],
  terminals: Record<string, TerminalSession>,
  hosts: readonly Host[] = [],
): SidebarWorkspaceTab[] {
  const tabs: SidebarWorkspaceTab[] = [];
  const seenTerminalIds = new Set<string>();
  const seenDraftIds = new Set<string>();
  const seenEmptyIds = new Set<string>();
  let hasSettings = false;

  const addTerminal = (id: string) => {
    if (seenTerminalIds.has(id)) return;
    seenTerminalIds.add(id);
    const terminal = terminals[id];
    tabs.push(terminal
      ? { kind: 'terminal', id, title: terminalTabTitle(terminal, hosts) }
      : { kind: 'terminal', id, title: 'Terminal unavailable', disabled: true });
  };
  const addSettings = (category: unknown) => {
    if (hasSettings) return;
    hasSettings = true;
    tabs.push({ kind: 'settings', id: 'settings', title: settingsTabTitle(category) });
  };

  for (const pane of panes) {
    const terminalIds = new Set(
      Array.isArray(pane.openTerminalIds)
        ? pane.openTerminalIds.filter((id): id is string => typeof id === 'string')
        : [],
    );
    const draftIds = new Set(
      Array.isArray(pane.openDraftIds)
        ? pane.openDraftIds.filter((id): id is string => typeof id === 'string')
        : [],
    );
    const emptyIds = new Set(
      Array.isArray(pane.openEmptyIds)
        ? pane.openEmptyIds.filter((id): id is string => typeof id === 'string')
        : [],
    );
    const drafts = pane.taskDrafts && typeof pane.taskDrafts === 'object'
      ? pane.taskDrafts as Record<string, unknown>
      : {};
    const addDraft = (id: string) => {
      if (seenDraftIds.has(id)) return;
      seenDraftIds.add(id);
      const draft = drafts[id];
      const title = draft && typeof draft === 'object' && typeof (draft as { title?: unknown }).title === 'string'
        ? (draft as { title: string }).title.trim()
        : '';
      tabs.push({ kind: 'draft', id, title: title || 'New chat' });
    };
    const addEmpty = (id: string) => {
      if (seenEmptyIds.has(id)) return;
      seenEmptyIds.add(id);
      tabs.push({ kind: 'empty', id, title: 'New tab' });
    };
    const ordered = Array.isArray(pane.tabOrder) ? pane.tabOrder : [];
    for (const item of ordered) {
      if (!item || typeof item !== 'object') continue;
      const tab = item as TabOrderItem;
      if (tab.kind === 'terminal' && typeof tab.id === 'string' && terminalIds.has(tab.id)) addTerminal(tab.id);
      else if (tab.kind === 'settings' && pane.settingsOpen === true) addSettings(pane.settingsCategory);
      else if (tab.kind === 'draft' && typeof tab.id === 'string' && draftIds.has(tab.id)) addDraft(tab.id);
      else if (tab.kind === 'empty' && typeof tab.id === 'string' && emptyIds.has(tab.id)) addEmpty(tab.id);
    }
    for (const id of terminalIds) addTerminal(id);
    for (const id of draftIds) addDraft(id);
    for (const id of emptyIds) addEmpty(id);
    if (pane.settingsOpen === true) addSettings(pane.settingsCategory);
  }
  return tabs;
}
