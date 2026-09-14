import { writable } from 'svelte/store';

export type AppThemeId =
  | 'monitter'
  | 'catppuccin'
  | 'dracula'
  | 'gruvbox'
  | 'nord'
  | 'one'
  | 'solarized'
  | 'tokyo-night'
  | 'wombat'
  | 'xterm';

export type AppThemePalette = {
  accent: string;
  paper: string;
  sidebar: string;
  panel: string;
  line: string;
  soft: string;
  code: string;
  ink: string;
  muted: string;
};

export type AppThemePreset = {
  id: AppThemeId;
  label: string;
  light: AppThemePalette;
  dark: AppThemePalette;
};

export type AppThemeSelection = {
  light: AppThemeId;
  dark: AppThemeId;
  accent: string | null;
};

export const appThemes: readonly AppThemePreset[] = [
  {
    id: 'monitter', label: 'Monitter',
    light: { accent:'#00A8F0', paper:'#F7F7F7', sidebar:'#F0F0F0', panel:'#FFFFFF', line:'#D8D8D8', soft:'#EEEEEE', code:'#E8E8E8', ink:'#252525', muted:'#747474' },
    dark: { accent:'#00A8F0', paper:'#151515', sidebar:'#0F0F0F', panel:'#202020', line:'#3D3D3D', soft:'#272727', code:'#292929', ink:'#EEEEEE', muted:'#A4A4A4' },
  },
  {
    id: 'catppuccin', label: 'Catppuccin',
    light: { accent:'#1E66F5', paper:'#EFF1F5', sidebar:'#E6E9EF', panel:'#FFFFFF', line:'#BCC0CC', soft:'#E6E9EF', code:'#DCE0E8', ink:'#4C4F69', muted:'#6C6F85' },
    dark: { accent:'#89B4FA', paper:'#1E1E2E', sidebar:'#181825', panel:'#313244', line:'#45475A', soft:'#29293B', code:'#313244', ink:'#CDD6F4', muted:'#A6ADC8' },
  },
  {
    id: 'dracula', label: 'Dracula',
    light: { accent:'#7C4DBD', paper:'#F8F8F2', sidebar:'#EEEFF4', panel:'#FFFFFF', line:'#C7C8D1', soft:'#EDEDF3', code:'#E4E4EB', ink:'#282A36', muted:'#62657A' },
    dark: { accent:'#BD93F9', paper:'#282A36', sidebar:'#21222C', panel:'#343746', line:'#44475A', soft:'#30323F', code:'#343746', ink:'#F8F8F2', muted:'#AAB0C5' },
  },
  {
    id: 'gruvbox', label: 'Gruvbox',
    light: { accent:'#79740E', paper:'#FBF1C7', sidebar:'#F2E5BC', panel:'#FFF8D8', line:'#D5C4A1', soft:'#EBDBB2', code:'#E6D5AE', ink:'#3C3836', muted:'#665C54' },
    dark: { accent:'#B8BB26', paper:'#282828', sidebar:'#1D2021', panel:'#32302F', line:'#504945', soft:'#3C3836', code:'#3C3836', ink:'#EBDBB2', muted:'#A89984' },
  },
  {
    id: 'nord', label: 'Nord',
    light: { accent:'#5E81AC', paper:'#ECEFF4', sidebar:'#E5E9F0', panel:'#FFFFFF', line:'#D8DEE9', soft:'#E5E9F0', code:'#D8DEE9', ink:'#2E3440', muted:'#4C566A' },
    dark: { accent:'#88C0D0', paper:'#2E3440', sidebar:'#242933', panel:'#3B4252', line:'#4C566A', soft:'#353C4A', code:'#3B4252', ink:'#ECEFF4', muted:'#D8DEE9' },
  },
  {
    id: 'one', label: 'One',
    light: { accent:'#4078F2', paper:'#FAFAFA', sidebar:'#F0F0F0', panel:'#FFFFFF', line:'#D3D3D3', soft:'#EAEAEB', code:'#E5E5E6', ink:'#383A42', muted:'#696C77' },
    dark: { accent:'#61AFEF', paper:'#21252B', sidebar:'#181A1F', panel:'#282C34', line:'#3E4451', soft:'#2C313C', code:'#2C313C', ink:'#ABB2BF', muted:'#8B919D' },
  },
  {
    id: 'solarized', label: 'Solarized',
    light: { accent:'#268BD2', paper:'#FDF6E3', sidebar:'#EEE8D5', panel:'#FFFBED', line:'#D6CEBA', soft:'#EEE8D5', code:'#E7E0CB', ink:'#586E75', muted:'#657B83' },
    dark: { accent:'#2AA198', paper:'#002B36', sidebar:'#001E27', panel:'#073642', line:'#586E75', soft:'#073642', code:'#073642', ink:'#93A1A1', muted:'#839496' },
  },
  {
    id: 'tokyo-night', label: 'Tokyo Night',
    light: { accent:'#2E7DE9', paper:'#E1E2E7', sidebar:'#D5D6DB', panel:'#F1F2F4', line:'#B4B5B9', soft:'#DCDDE2', code:'#D5D6DB', ink:'#343B58', muted:'#6172B0' },
    dark: { accent:'#7AA2F7', paper:'#1A1B26', sidebar:'#16161E', panel:'#24283B', line:'#414868', soft:'#292E42', code:'#24283B', ink:'#C0CAF5', muted:'#A9B1D6' },
  },
  {
    id: 'wombat', label: 'Wombat',
    light: { accent:'#5C7A29', paper:'#F2EFE8', sidebar:'#E8E4DC', panel:'#FAF8F2', line:'#CBC5BA', soft:'#E2DDD4', code:'#D9D4CB', ink:'#3A3834', muted:'#6F6B64' },
    dark: { accent:'#B1E969', paper:'#171717', sidebar:'#101010', panel:'#242424', line:'#444444', soft:'#2D2D2D', code:'#2D2D2D', ink:'#DEDACF', muted:'#AAA69E' },
  },
  {
    id: 'xterm', label: 'XTerm',
    light: { accent:'#0000EE', paper:'#FFFFFF', sidebar:'#F3F3F3', panel:'#FFFFFF', line:'#CCCCCC', soft:'#EBEBEB', code:'#E3E3E3', ink:'#000000', muted:'#555555' },
    dark: { accent:'#5C5CFF', paper:'#000000', sidebar:'#080808', panel:'#111111', line:'#444444', soft:'#1A1A1A', code:'#222222', ink:'#FFFFFF', muted:'#BEBEBE' },
  },
];

export const DEFAULT_APP_THEME: AppThemeSelection = { light: 'monitter', dark: 'monitter', accent: null };
const storageKey = 'monitter.appearance.app-theme-pair.v2';
const legacyStorageKey = 'monitter.appearance.app-theme.v1';
const colourPattern = /^#[0-9a-f]{6}$/i;

export function normalizeAppTheme(value: unknown): AppThemeId {
  return appThemes.some(theme => theme.id === value) ? value as AppThemeId : 'monitter';
}

export function normalizeAppThemeSelection(value: unknown): AppThemeSelection {
  const candidate = value && typeof value === 'object' ? value as Partial<AppThemeSelection> : {};
  return {
    light: normalizeAppTheme(candidate.light),
    dark: normalizeAppTheme(candidate.dark),
    accent: typeof candidate.accent === 'string' && colourPattern.test(candidate.accent) ? candidate.accent : null,
  };
}

function legacyTheme(value: string | null): Partial<AppThemeSelection> {
  if (!value || value === 'custom' || value === 'monitter') return {};
  if (value === 'catppuccin-latte') return { light: 'catppuccin' };
  const aliases: Record<string, AppThemeId> = {
    'catppuccin-mocha':'catppuccin', 'gruvbox-dark':'gruvbox', 'one-dark':'one', 'solarized-dark':'solarized',
    dracula:'dracula', nord:'nord', 'tokyo-night':'tokyo-night', wombat:'wombat',
  };
  return aliases[value] ? { dark: aliases[value] } : {};
}

function loadAppTheme(): AppThemeSelection {
  if (typeof window === 'undefined') return { ...DEFAULT_APP_THEME };
  try {
    const stored = window.localStorage.getItem(storageKey);
    if (stored) return normalizeAppThemeSelection(JSON.parse(stored));
    return normalizeAppThemeSelection({ ...DEFAULT_APP_THEME, ...legacyTheme(window.localStorage.getItem(legacyStorageKey)) });
  } catch { return { ...DEFAULT_APP_THEME }; }
}

export function appThemePreset(id: AppThemeId): AppThemePreset {
  return appThemes.find(theme => theme.id === id) ?? appThemes[0];
}

export function mixThemeColour(background: string, accent: string, amount: number): string {
  const channels = (value: string) => [1, 3, 5].map(offset => Number.parseInt(value.slice(offset, offset + 2), 16));
  const base = channels(background), highlight = channels(accent), weight = Math.min(1, Math.max(0, amount));
  return `#${base.map((channel, index) => Math.round(channel * (1 - weight) + highlight[index] * weight).toString(16).padStart(2, '0')).join('')}`.toUpperCase();
}

/** Palette selection is presentation state, so each browser/native client can differ. */
export const appTheme = writable<AppThemeSelection>(loadAppTheme());

export function setAppTheme(mode: 'light' | 'dark', value: AppThemeId): void {
  appTheme.update(current => {
    const next = { ...current, [mode]: normalizeAppTheme(value) };
    if (typeof window !== 'undefined') {
      try { window.localStorage.setItem(storageKey, JSON.stringify(next)); } catch { /* Live selection still applies. */ }
    }
    return next;
  });
}

export function setAppThemeAccent(value: string | null): void {
  appTheme.update(current => {
    const next = { ...current, accent: typeof value === 'string' && colourPattern.test(value) ? value : null };
    if (typeof window !== 'undefined') {
      try { window.localStorage.setItem(storageKey, JSON.stringify(next)); } catch { /* Live selection still applies. */ }
    }
    return next;
  });
}

export function resetAppTheme(): void {
  const next = { ...DEFAULT_APP_THEME };
  appTheme.set(next);
  if (typeof window === 'undefined') return;
  try { window.localStorage.setItem(storageKey, JSON.stringify(next)); } catch { /* Live selection still applies. */ }
}
