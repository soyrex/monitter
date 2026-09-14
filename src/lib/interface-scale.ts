import { writable, type Writable } from 'svelte/store';

export type ViewerType = 'desktop' | 'mobile';

export const DEFAULT_INTERFACE_SCALE = 125;
export const MIN_INTERFACE_SCALE = 80;
export const MAX_INTERFACE_SCALE = 200;

const keyPrefix = 'monitter.interface-scale.v1';
const stores: Record<ViewerType, Writable<number>> = {
  desktop: writable(DEFAULT_INTERFACE_SCALE),
  mobile: writable(DEFAULT_INTERFACE_SCALE),
};

export function interfaceScaleStorageKey(viewer: ViewerType): string {
  return `${keyPrefix}:${viewer}`;
}

export function normalizeInterfaceScale(value: unknown): number {
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) return DEFAULT_INTERFACE_SCALE;
  return Math.min(MAX_INTERFACE_SCALE, Math.max(MIN_INTERFACE_SCALE, Math.round(parsed / 5) * 5));
}

function storedScale(viewer: ViewerType): number | null {
  if (typeof localStorage === 'undefined') return null;
  try {
    const raw = localStorage.getItem(interfaceScaleStorageKey(viewer));
    if (raw === null) return null;
    const parsed = Number(raw);
    return Number.isFinite(parsed) && parsed >= MIN_INTERFACE_SCALE && parsed <= MAX_INTERFACE_SCALE
      ? normalizeInterfaceScale(parsed)
      : null;
  } catch {
    return null;
  }
}

export function interfaceScaleStore(viewer: ViewerType): Writable<number> {
  return stores[viewer];
}

/** Import the former shared scale once, then keep this viewer type independent. */
export function seedViewerInterfaceScale(viewer: ViewerType, legacyScale: unknown): number {
  const existing = storedScale(viewer);
  const next = existing ?? normalizeInterfaceScale(legacyScale);
  stores[viewer].set(next);
  if (existing === null) {
    try { localStorage.setItem(interfaceScaleStorageKey(viewer), String(next)); }
    catch { /* The in-memory preference still applies. */ }
  }
  return next;
}

export function setViewerInterfaceScale(viewer: ViewerType, value: unknown): number {
  const next = normalizeInterfaceScale(value);
  stores[viewer].set(next);
  try { localStorage.setItem(interfaceScaleStorageKey(viewer), String(next)); }
  catch { /* The in-memory preference still applies. */ }
  return next;
}

export function watchViewerInterfaceScales(): () => void {
  if (typeof window === 'undefined') return () => {};
  for (const viewer of ['desktop', 'mobile'] as const) {
    const saved = storedScale(viewer);
    if (saved !== null) stores[viewer].set(saved);
  }
  const sync = (event: StorageEvent) => {
    for (const viewer of ['desktop', 'mobile'] as const) {
      if (event.key === interfaceScaleStorageKey(viewer)) {
        const saved = storedScale(viewer);
        if (saved !== null) stores[viewer].set(saved);
      }
    }
  };
  window.addEventListener('storage', sync);
  return () => window.removeEventListener('storage', sync);
}
