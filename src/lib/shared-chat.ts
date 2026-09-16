import type { Snapshot } from './types';

export const sharedAppearanceVariables = [
  '--paper', '--sidebar', '--panel', '--line', '--soft', '--code', '--ink', '--muted',
  '--accent', '--accent-rgb', '--accent-ink', '--on-accent', '--mono', '--interface-font',
  '--chat-font', '--chat-font-size', '--chat-line-height', '--interface-font-ratio', '--chat-font-ratio',
] as const;
export interface SharedChatAppearance {
  theme: 'light' | 'dark';
  variables: Partial<Record<typeof sharedAppearanceVariables[number], string>>;
}
export interface SharedChatDetails {
  primary: { name: string; role: 'primary user' };
  visitor: { name: string; role: 'visitor' };
  appearance?: SharedChatAppearance;
  uploads?: { maxFileBytes: number; maxFiles: number };
}
export type SharedChatSnapshot = Snapshot & { sharing?: SharedChatDetails };

/** Stable participant hues keep owner and visitor views consistent. */
export function participantColour(name: string): string {
  let hash = 2166136261;
  for (const char of name.trim().toLocaleLowerCase()) hash = Math.imul(hash ^ char.codePointAt(0)!, 16777619);
  return `hsl(${(hash >>> 0) % 360} 55% 52%)`;
}

const colourVariables = new Set(['--paper', '--sidebar', '--panel', '--line', '--soft', '--code', '--ink', '--muted', '--accent', '--accent-ink', '--on-accent']);
function safeAppearanceValue(property: string, value: unknown): value is string {
  if (typeof value !== 'string' || !value.trim() || value.length > 512 || /[;{}<>]|url\s*\(|var\s*\(/i.test(value)) return false;
  if (colourVariables.has(property)) return CSS.supports('color', value);
  if (property === '--accent-rgb') return /^\s*[\d.]+\s*,\s*[\d.]+\s*,\s*[\d.]+\s*$/.test(value);
  if (property.endsWith('-font') || property === '--mono') return CSS.supports('font-family', value);
  if (property === '--chat-font-size') return /^\d+(\.\d+)?px$/.test(value) && parseFloat(value) >= 8 && parseFloat(value) <= 48;
  return /^\d+(\.\d+)?$/.test(value) && Number(value) > 0 && Number(value) <= 4;
}

/** Capture resolved tokens rather than a skin ID: tint, contrast and custom fonts
 * are local owner preferences and are not present in the native Snapshot. */
export function captureSharedAppearance(): SharedChatAppearance | undefined {
  if (typeof document === 'undefined') return undefined;
  const root = document.documentElement;
  const computed = getComputedStyle(root);
  const theme = root.dataset.theme === 'dark' || (root.dataset.theme === 'system' && matchMedia('(prefers-color-scheme: dark)').matches) ? 'dark' : 'light';
  const variables: SharedChatAppearance['variables'] = {};
  for (const property of sharedAppearanceVariables) {
    const value = computed.getPropertyValue(property).trim();
    if (safeAppearanceValue(property, value)) variables[property] = value;
  }
  return Object.keys(variables).length ? { theme, variables } : undefined;
}

export function applySharedAppearance(value: unknown): boolean {
  if (typeof document === 'undefined' || !value || typeof value !== 'object') return false;
  const candidate = value as SharedChatAppearance;
  if (!['light', 'dark'].includes(candidate.theme) || !candidate.variables || typeof candidate.variables !== 'object') return false;
  const root = document.documentElement;
  for (const property of sharedAppearanceVariables) {
    const token = candidate.variables[property];
    if (safeAppearanceValue(property, token)) root.style.setProperty(property, token.trim());
  }
  root.dataset.theme = candidate.theme;
  root.style.colorScheme = candidate.theme;
  return true;
}
