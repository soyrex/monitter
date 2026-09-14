import { insertTab, normalizeTabOrder, type TabKey } from '$lib/tab-order';

/**
 * State that belongs to one rendered pane, rather than the workspace root.
 *
 * A pane can be promoted, moved between layouts, or restored from a different
 * workspace. Keeping its ordering and selection history together prevents the
 * root layout coordinator from becoming the implicit owner of those details.
 */
export class PaneLocalState {
  order = $state<TabKey[]>([]);
  private selectionHistory = new Map<string, TabKey[]>();

  ordered(available: readonly TabKey[]) {
    return normalizeTabOrder(this.order, available);
  }

  restore(order: readonly TabKey[], available: readonly TabKey[]) {
    this.order = normalizeTabOrder(order, available);
  }

  remember(tab: TabKey, available: readonly TabKey[], before?: TabKey) {
    const current = this.ordered(available);
    this.order = !before && current.some(item => item.kind === tab.kind && item.id === tab.id)
      ? current
      : insertTab(current, tab, before);
  }

  forget(tab: TabKey) {
    this.order = this.order.filter(item => item.kind !== tab.kind || item.id !== tab.id);
  }

  reorder(tab: TabKey, available: readonly TabKey[], before?: TabKey) {
    this.order = insertTab(this.ordered(available), tab, before);
  }

  swap(tab: TabKey, available: readonly TabKey[], direction: 1 | -1) {
    const ordered = this.ordered(available);
    const index = ordered.findIndex(item => item.kind === tab.kind && item.id === tab.id);
    const target = index + direction;
    if (index < 0 || target < 0 || target >= ordered.length) return false;
    [ordered[index], ordered[target]] = [ordered[target], ordered[index]];
    this.order = ordered;
    return true;
  }

  recordSelection(workspace: string, tab: TabKey | null) {
    if (!tab) return;
    const history = this.selectionHistory.get(workspace) ?? [];
    this.selectionHistory.set(workspace, [
      ...history.filter(item => item.kind !== tab.kind || item.id !== tab.id),
      tab,
    ]);
  }

  mostRecentRemaining(workspace: string, remaining: readonly TabKey[]) {
    const history = this.selectionHistory.get(workspace) ?? [];
    return [...history].reverse().find(item => remaining.some(candidate =>
      candidate.kind === item.kind && candidate.id === item.id,
    ));
  }
}

export function createPaneLocalState() {
  return new PaneLocalState();
}
