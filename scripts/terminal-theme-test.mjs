import assert from 'node:assert/strict';
import { DEFAULT_TERMINAL_THEME, normalizeTerminalTheme, terminalPalette, terminalThemes } from '../src/lib/terminal-theme.ts';

assert.equal(terminalThemes.length, 10);
assert.deepEqual(terminalThemes.map(theme=>theme.label), [
  'Monitter', 'Catppuccin Mocha', 'Dracula', 'Gruvbox Dark', 'Molokai',
  'Nord', 'One Dark', 'Solarized Dark', 'Wombat', 'XTerm',
]);
assert.equal(normalizeTerminalTheme('catppuccin-mocha'), 'catppuccin-mocha');
assert.equal(normalizeTerminalTheme('unknown'), DEFAULT_TERMINAL_THEME);
for (const theme of terminalThemes) {
  const value = terminalPalette(theme.id);
  for (const [key, colour] of Object.entries(value)) {
    assert.match(colour, /^#[0-9a-f]{6}(?:[0-9a-f]{2})?$/i, `${theme.label} ${key} is a hex colour`);
  }
  assert.notEqual(value.background.toLowerCase(), value.foreground.toLowerCase());
}
console.log('ten offline terminal themes expose complete ANSI palettes');
