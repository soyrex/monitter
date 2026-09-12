import { readFile } from 'node:fs/promises';
import { strict as assert } from 'node:assert';

const [types, pane, model, backend, contract] = await Promise.all([
  readFile(new URL('../src/lib/types.ts', import.meta.url), 'utf8'),
  readFile(new URL('../src/lib/components/SettingsPane.svelte', import.meta.url), 'utf8'),
  readFile(new URL('../src-tauri/src/model.rs', import.meta.url), 'utf8'),
  readFile(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8'),
  readFile(new URL('../docs/CONTRACT.md', import.meta.url), 'utf8'),
]);
assert.ok(types.includes("tabStyle?: 'classic' | 'modern';"), 'TS settings boundary must expose only the two tab styles');
assert.ok(pane.includes('aria-label="Tab style"') && pane.includes("settings.tabStyle ?? 'classic'") && pane.includes("save({ tabStyle:"), 'Appearance UI must default to classic and save either segmented choice');
assert.ok(model.includes('#[serde(default = "default_tab_style")]') && model.includes('fn default_tab_style() -> String') && model.includes('"classic".into()'), 'Older state must deserialize to classic');
assert.ok(model.includes('fn tab_style_round_trips_through_settings_json()'), 'Modern tab style must round-trip through persisted JSON');
assert.ok(backend.includes('"classic" | "modern"') && backend.includes('Tab style must be classic or modern.'), 'Backend must reject unsupported tab styles on desktop and LAN saves');
assert.ok(contract.includes('`tabStyle` is `classic` or `modern`, defaulting to `classic`'), 'Desktop/LAN appearance contract must document the default');
console.log('tab style settings contract: TS/UI toggle, classic migration default, camelCase round-trip, backend validation, and shared LAN documentation');
