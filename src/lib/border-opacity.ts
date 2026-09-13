import { writable } from 'svelte/store';

export const DEFAULT_BORDER_OPACITY = 100;
const storageKey = 'monitter.appearance.border-opacity.v1';

export function normalizeBorderOpacity(value: number): number {
  return Number.isFinite(value) ? Math.max(0, Math.min(100, Math.round(value))) : DEFAULT_BORDER_OPACITY;
}

function loadBorderOpacity(): number {
  try {
    const saved = localStorage.getItem(storageKey);
    return saved === null ? DEFAULT_BORDER_OPACITY : normalizeBorderOpacity(Number(saved));
  } catch { return DEFAULT_BORDER_OPACITY; }
}

/** Client-local so browser and native windows can tune their chrome independently. */
export const borderOpacity = writable(loadBorderOpacity());

export function setBorderOpacity(value: number): void {
  const next = normalizeBorderOpacity(value);
  borderOpacity.set(next);
  try { localStorage.setItem(storageKey, String(next)); } catch { /* Live preview still applies. */ }
}
