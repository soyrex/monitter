import assert from 'node:assert/strict';

const storage = new Map();
globalThis.localStorage = {
  getItem: key => storage.get(key) ?? null,
  setItem: (key, value) => { storage.set(key, value); },
};
globalThis.window = { localStorage: globalThis.localStorage };
const { applyOwnerAppearance, currentOwnerAppearance } = await import('../src/lib/owner-appearance.ts');

const chosen = {
  version: 1,
  appTheme: { light: 'catppuccin', dark: 'nord', accent: '#abcdef', contrast: 37 },
  surfaceTint: 18,
  terminalTheme: 'gruvbox-dark',
};
assert.deepEqual(applyOwnerAppearance(chosen), chosen);
assert.deepEqual(currentOwnerAppearance(), chosen);
assert.equal(JSON.parse(storage.get('monitter.appearance.app-theme-pair.v2')).dark, 'nord');
assert.equal(storage.get('monitter.appearance.surface-tint.v1'), '18');
assert.equal(storage.get('monitter.appearance.terminal-theme.v1'), 'gruvbox-dark');

// Relay presentation data is untrusted even after authentication; invalid
// fields must not reach CSS or local storage as arbitrary values.
assert.deepEqual(applyOwnerAppearance({
  version: 1,
  appTheme: { light: 'unknown', dark: 'nord', accent: 'red', contrast: 900 },
  surfaceTint: -10,
  terminalTheme: 'unknown',
}), {
  version: 1,
  appTheme: { light: 'monitter', dark: 'nord', accent: null, contrast: 100 },
  surfaceTint: 0,
  terminalTheme: 'monitter',
});
assert.throws(() => applyOwnerAppearance({ ...chosen, version: 2 }), /invalid/i);
assert.throws(() => applyOwnerAppearance({ ...chosen, surfaceTint: '18' }), /invalid/i);
console.log('Owner appearance sync normalization assertions passed.');
