import { writable } from 'svelte/store';

export const DEFAULT_SURFACE_TINT = 5;
const storageKey = 'monitter.appearance.surface-tint.v1';
export function normalizeSurfaceTint(value: number): number {
  return Number.isFinite(value) ? Math.max(0, Math.min(50, Math.round(value))) : DEFAULT_SURFACE_TINT;
}
function loadTint(): number {
  try {
    const saved = localStorage.getItem(storageKey);
    return saved === null ? DEFAULT_SURFACE_TINT : normalizeSurfaceTint(Number(saved));
  } catch { return DEFAULT_SURFACE_TINT; }
}
/** Client-local appearance preference; compatible with older desktop backends. */
export const surfaceTint = writable(loadTint());
export function setSurfaceTint(value: number): void {
  const next = normalizeSurfaceTint(value);
  surfaceTint.set(next);
  try { localStorage.setItem(storageKey, String(next)); } catch { /* Keep the live setting when storage is unavailable. */ }
}
