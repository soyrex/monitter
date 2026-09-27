import { get } from 'svelte/store';
import { appTheme, normalizeAppThemeSelection, setAppTheme, setAppThemeAccent, setAppThemeContrast, type AppThemeSelection } from './app-theme';
import { normalizeSurfaceTint, setSurfaceTint, surfaceTint } from './surface-tint';
import { normalizeTerminalTheme, setTerminalTheme, terminalTheme, type TerminalThemeId } from './terminal-theme';

/** Small, presentation-only state not stored in the desktop Snapshot. */
export interface OwnerAppearance {
  version: 1;
  appTheme: AppThemeSelection;
  surfaceTint: number;
  terminalTheme: TerminalThemeId;
}

export function currentOwnerAppearance(): OwnerAppearance {
  return {
    version: 1,
    appTheme: get(appTheme),
    surfaceTint: get(surfaceTint),
    terminalTheme: get(terminalTheme),
  };
}

/** Validate the untrusted relay value before it reaches CSS or local storage. */
export function applyOwnerAppearance(value: unknown): OwnerAppearance {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('Owner appearance data is invalid.');
  const input = value as Record<string, unknown>;
  if (input.version !== 1 || !input.appTheme || typeof input.appTheme !== 'object' || Array.isArray(input.appTheme)
    || typeof input.surfaceTint !== 'number' || !Number.isFinite(input.surfaceTint)
    || typeof input.terminalTheme !== 'string') throw new Error('Owner appearance data is invalid.');
  const selected = normalizeAppThemeSelection(input.appTheme);
  const tint = normalizeSurfaceTint(input.surfaceTint);
  const terminal = normalizeTerminalTheme(input.terminalTheme);
  setAppTheme('light', selected.light);
  setAppTheme('dark', selected.dark);
  setAppThemeAccent(selected.accent);
  setAppThemeContrast(selected.contrast);
  setSurfaceTint(tint);
  setTerminalTheme(terminal);
  return { version: 1, appTheme: selected, surfaceTint: tint, terminalTheme: terminal };
}
