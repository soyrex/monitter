import { writable } from 'svelte/store';

export type TerminalThemeId =
  | 'monitter'
  | 'catppuccin-mocha'
  | 'dracula'
  | 'gruvbox-dark'
  | 'molokai'
  | 'nord'
  | 'one-dark'
  | 'solarized-dark'
  | 'wombat'
  | 'xterm';

export type TerminalPalette = {
  background: string;
  foreground: string;
  cursor: string;
  selectionBackground: string;
  black: string;
  red: string;
  green: string;
  yellow: string;
  blue: string;
  magenta: string;
  cyan: string;
  white: string;
  brightBlack: string;
  brightRed: string;
  brightGreen: string;
  brightYellow: string;
  brightBlue: string;
  brightMagenta: string;
  brightCyan: string;
  brightWhite: string;
};

type ThemeOption = { id: TerminalThemeId; label: string; palette: TerminalPalette };

const palette = (background: string, foreground: string, cursor: string, colours: readonly string[]): TerminalPalette => ({
  background, foreground, cursor, selectionBackground: `${colours[8]}99`,
  black: colours[0], red: colours[1], green: colours[2], yellow: colours[3],
  blue: colours[4], magenta: colours[5], cyan: colours[6], white: colours[7],
  brightBlack: colours[8], brightRed: colours[9], brightGreen: colours[10], brightYellow: colours[11],
  brightBlue: colours[12], brightMagenta: colours[13], brightCyan: colours[14], brightWhite: colours[15],
});

// Palettes are adapted from the terminal-community Gogh catalogue. Keeping the
// values here makes terminal startup deterministic and available offline.
export const terminalThemes: readonly ThemeOption[] = [
  { id: 'monitter', label: 'Monitter', palette: palette('#090b0d', '#e5e7eb', '#77b58b', ['#090b0d','#e05f65','#77b58b','#d8b15d','#5c9ded','#b980d9','#5cbec9','#d8dadd','#596168','#ff777d','#8dcc9f','#edc873','#79b7ff','#d39bed','#77d7df','#ffffff']) },
  { id: 'catppuccin-mocha', label: 'Catppuccin Mocha', palette: palette('#1E1E2E', '#CDD6F4', '#F5E0DC', ['#45475A','#F38BA8','#A6E3A1','#F9E2AF','#89B4FA','#F5C2E7','#94E2D5','#A6ADC8','#585B70','#F37799','#89D88B','#EBD391','#74A8FC','#F2AEDE','#6BD7CA','#BAC2DE']) },
  { id: 'dracula', label: 'Dracula', palette: palette('#282A36', '#F8F8F2', '#F8F8F2', ['#21222C','#FF5555','#50FA7B','#F1FA8C','#BD93F9','#FF79C6','#8BE9FD','#F8F8F2','#6272A4','#FF6E6E','#69FF94','#FFFFA5','#D6ACFF','#FF92DF','#A4FFFF','#FFFFFF']) },
  { id: 'gruvbox-dark', label: 'Gruvbox Dark', palette: palette('#282828', '#EBDBB2', '#EBDBB2', ['#282828','#CC241D','#98971A','#D79921','#458588','#B16286','#689D6A','#A89984','#928374','#FB4934','#B8BB26','#FABD2F','#83A598','#D3869B','#8EC07C','#EBDBB2']) },
  { id: 'molokai', label: 'Molokai', palette: palette('#121212', '#BBBBBB', '#BBBBBB', ['#121212','#FA2573','#98E123','#DFD460','#1080D0','#8700FF','#43A8D0','#BBBBBB','#555555','#F6669D','#B1E05F','#FFF26D','#00AFFF','#AF87FF','#51CEFF','#FFFFFF']) },
  { id: 'nord', label: 'Nord', palette: palette('#2E3440', '#D8DEE9', '#ECEFF4', ['#3B4252','#BF616A','#A3BE8C','#EBCB8B','#81A1C1','#B48EAD','#88C0D0','#E5E9F0','#4C566A','#BF616A','#A3BE8C','#EBCB8B','#81A1C1','#B48EAD','#8FBCBB','#ECEFF4']) },
  { id: 'one-dark', label: 'One Dark', palette: palette('#21252B', '#ABB2BF', '#ABB2BF', ['#21252B','#E06C75','#98C379','#E5C07B','#61AFEF','#C678DD','#56B6C2','#ABB2BF','#767676','#E06C75','#98C379','#E5C07B','#61AFEF','#C678DD','#56B6C2','#ABB2BF']) },
  { id: 'solarized-dark', label: 'Solarized Dark', palette: palette('#001E27', '#708284', '#708284', ['#002831','#D11C24','#738A05','#A57706','#2176C7','#C61C6F','#259286','#EAE3CB','#001E27','#BD3613','#475B62','#536870','#708284','#5956BA','#819090','#FCF4DC']) },
  { id: 'wombat', label: 'Wombat', palette: palette('#171717', '#DEDACF', '#BBBBBB', ['#000000','#FF615A','#B1E969','#EBD99C','#5DA9F6','#E86AFF','#82FFF7','#DEDACF','#313131','#F58C80','#DDF88F','#EEE5B2','#A5C7FF','#DDAAFF','#B7FFF9','#FFFFFF']) },
  { id: 'xterm', label: 'XTerm', palette: palette('#000000', '#FFFFFF', '#FFFFFF', ['#000000','#CD0000','#00CD00','#CDCD00','#0000EE','#CD00CD','#00CDCD','#E5E5E5','#7F7F7F','#FF0000','#00FF00','#FFFF00','#5C5CFF','#FF00FF','#00FFFF','#FFFFFF']) },
];

export const DEFAULT_TERMINAL_THEME: TerminalThemeId = 'monitter';
const storageKey = 'monitter.appearance.terminal-theme.v1';

export function normalizeTerminalTheme(value: unknown): TerminalThemeId {
  return terminalThemes.some(theme => theme.id === value) ? value as TerminalThemeId : DEFAULT_TERMINAL_THEME;
}

function loadTerminalTheme(): TerminalThemeId {
  if (typeof window === 'undefined') return DEFAULT_TERMINAL_THEME;
  try { return normalizeTerminalTheme(window.localStorage.getItem(storageKey)); }
  catch { return DEFAULT_TERMINAL_THEME; }
}

export function terminalPalette(id: TerminalThemeId): TerminalPalette {
  return terminalThemes.find(theme => theme.id === id)?.palette ?? terminalThemes[0].palette;
}

function applyTerminalSurface(id: TerminalThemeId): void {
  if (typeof document === 'undefined') return;
  const root = document.documentElement;
  const selected = terminalPalette(id);
  root.dataset.terminalTheme = id;
  root.style.setProperty('--terminal-background', selected.background);
  root.style.setProperty('--terminal-foreground', selected.foreground);
}

/** Client-local so native, browser and remote views can keep distinct terminal palettes. */
export const terminalTheme = writable<TerminalThemeId>(loadTerminalTheme());
terminalTheme.subscribe(applyTerminalSurface);

export function setTerminalTheme(value: TerminalThemeId): void {
  const next = normalizeTerminalTheme(value);
  terminalTheme.set(next);
  if (typeof window === 'undefined') return;
  try { window.localStorage.setItem(storageKey, next); } catch { /* Live selection still applies. */ }
}
