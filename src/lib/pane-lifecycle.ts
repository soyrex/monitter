/** Minimal tab ownership needed to decide whether a pane may disappear. */
export type PaneTabState = {
  openTaskIds: readonly string[];
  openDraftIds: readonly string[];
  openChannelIds: readonly string[];
  openTerminalIds: readonly string[];
  openEmptyIds: readonly string[];
  settingsOpen: boolean;
};

export function paneTabCount(state: PaneTabState) {
  return state.openTaskIds.length + state.openDraftIds.length + state.openChannelIds.length
    + state.openTerminalIds.length + state.openEmptyIds.length + (state.settingsOpen ? 1 : 0);
}

/**
 * A requested collapse waits for an in-flight action, but never removes the
 * only workspace pane or a pane that acquired a tab while it was waiting.
 */
export function paneRemovalDecision(state: PaneTabState, paneCount: number, pending: boolean): 'keep' | 'defer' | 'remove' {
  if (paneCount <= 1 || paneTabCount(state) !== 0) return 'keep';
  return pending ? 'defer' : 'remove';
}

/** Remap references when removing main promotes another leaf into its ID. */
export function remapPromotedPaneId(id: string, removedId: string, promotedId: string | null) {
  if (promotedId && id === promotedId) return 'main';
  return id === removedId ? null : id;
}

export function remapQueuedPaneRemovals(ids: readonly string[], removedId: string, promotedId: string | null) {
  return [...new Set(ids.flatMap(id => {
    const mapped = remapPromotedPaneId(id, removedId, promotedId);
    return mapped ? [mapped] : [];
  }))];
}
