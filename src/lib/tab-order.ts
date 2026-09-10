export type TabKind = 'task' | 'draft' | 'channel' | 'terminal' | 'settings';
export type TabKey = { kind: TabKind; id: string };

export const tabKey = (tab: TabKey) => `${tab.kind}:${tab.id}`;

/** Keep persisted order valid when tabs were closed, restored, or moved. */
export function normalizeTabOrder(saved: readonly TabKey[], available: readonly TabKey[]): TabKey[] {
  const present = new Map(available.map(tab => [tabKey(tab), tab]));
  const ordered: TabKey[] = [];
  for (const tab of saved) {
    const current = present.get(tabKey(tab));
    if (current) { ordered.push(current); present.delete(tabKey(tab)); }
  }
  for (const tab of available) if (present.has(tabKey(tab))) ordered.push(tab);
  return ordered;
}

export function insertTab(order: readonly TabKey[], tab: TabKey, before?: TabKey): TabKey[] {
  const without = order.filter(current => tabKey(current) !== tabKey(tab));
  const index = before ? without.findIndex(current => tabKey(current) === tabKey(before)) : -1;
  return index < 0 ? [...without, tab] : [...without.slice(0, index), tab, ...without.slice(index)];
}
