import type { SidebarView } from './types';

export type SidebarViewClient = 'desktop' | 'web' | 'mobile';

const validViews = new Set<SidebarView>(['standard', 'activity', 'projects']);
const keyPrefix = 'monitter.sidebar-view.v2';

export function sidebarViewStorageKey(client: SidebarViewClient): string {
  return `${keyPrefix}:${client}`;
}

export function loadSidebarViewPreference(client: SidebarViewClient): SidebarView | null {
  // A tab keeps its own choice across reloads, even if another tab on the
  // same origin changes the durable default in localStorage.
  try {
    const value = typeof sessionStorage === 'undefined' ? null : sessionStorage.getItem(sidebarViewStorageKey(client));
    if (validViews.has(value as SidebarView)) return value as SidebarView;
  } catch { /* Fall back to the last durable client default. */ }
  if (typeof localStorage === 'undefined') return null;
  try {
    const value = localStorage.getItem(sidebarViewStorageKey(client));
    return validViews.has(value as SidebarView) ? value as SidebarView : null;
  } catch {
    return null;
  }
}

export function saveSidebarViewPreference(client: SidebarViewClient, view: SidebarView): boolean {
  if (!validViews.has(view)) return false;
  let saved = false;
  try {
    if (typeof sessionStorage !== 'undefined') {
      sessionStorage.setItem(sidebarViewStorageKey(client), view);
      saved = true;
    }
  } catch { /* The current in-memory choice still works. */ }
  try {
    if (typeof localStorage !== 'undefined') {
      localStorage.setItem(sidebarViewStorageKey(client), view);
      saved = true;
    }
  } catch {
    // Storage denial must not block local navigation.
  }
  return saved;
}
