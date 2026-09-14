import type { Agent, Channel, Task } from '$lib/types';
import type { PaneTabTransfer } from '$lib/panes';
import type { TabKey } from '$lib/tab-order';

/**
 * The root owns topology while individual pane surfaces own their local tab
 * state. Keeping that boundary explicit is the prerequisite for moving the
 * embedded surface out of AppSurface without weakening tab-transfer or
 * workspace-persistence semantics.
 */
export type PaneSurfaceHandle<State, Payload> = {
  focusExistingTab: (tab: TabKey) => boolean;
  selectRelativeTab: (direction: 1 | -1) => boolean;
  openAgentSettings: (draft: Agent) => void;
  openSettings: (category?: string) => void;
  openTerminalTab: (id: string) => void;
  newTerminal: () => Promise<void>;
  openEmptyTab: () => void;
  openTask: (task: Task, allowDuplicate?: boolean) => void;
  openChannel: (channel: Channel, allowDuplicate?: boolean) => void;
  openTaskComposer: (parentId?: string | null, agentId?: string | null, projectId?: string | null) => void;
  takeTab: (tab: PaneTabTransfer) => Payload | null;
  receiveTab: (payload: Payload, before?: TabKey) => void;
  reorderTab: (tab: PaneTabTransfer, before?: TabKey) => void;
  allTabs: () => PaneTabTransfer[];
  captureState: () => State;
  restoreState: (value: State) => void;
  closeActiveTab: () => void;
  swapActiveTab: (direction: 1 | -1) => void;
  toggleDetail: () => void;
  hasPending: () => boolean;
  attachNativeFiles: (paths: string[]) => Promise<void>;
};

type TabOwner = { allTabs: () => PaneTabTransfer[] };

/** Locate an existing tab once, preferring the focussed pane when applicable. */
export function findPaneTabOwner(
  paneIds: readonly string[],
  activePaneId: string,
  mainTabs: readonly PaneTabTransfer[],
  panes: Readonly<Record<string, TabOwner | undefined>>,
  matches: (tab: PaneTabTransfer) => boolean,
): string | null {
  const candidates = [activePaneId, ...paneIds.filter(id => id !== activePaneId)];
  for (const id of candidates) {
    const tabs = id === 'main' ? mainTabs : panes[id]?.allTabs() ?? [];
    if (tabs.some(matches)) return id;
  }
  return null;
}

export function paneTabMatches(tab: Pick<PaneTabTransfer, 'kind' | 'id'>, target: Pick<PaneTabTransfer, 'kind' | 'id'>) {
  return tab.kind === target.kind && tab.id === target.id;
}
