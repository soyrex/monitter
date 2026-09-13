import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const [types, model, backend, settings, surface, contract] = await Promise.all([
  readFile(new URL('../src/lib/types.ts', import.meta.url), 'utf8'),
  readFile(new URL('../src-tauri/src/model.rs', import.meta.url), 'utf8'),
  readFile(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8'),
  readFile(new URL('../src/lib/components/SettingsPane.svelte', import.meta.url), 'utf8'),
  readFile(new URL('../src/lib/components/AppSurface.svelte', import.meta.url), 'utf8'),
  readFile(new URL('../docs/CONTRACT.md', import.meta.url), 'utf8'),
]);

assert.ok(types.includes("interfaceDensity?: 'tight' | 'normal' | 'spacious';"), 'TS settings boundary must expose the three density values');
assert.ok(model.includes('default_interface_density') && model.includes('"normal".into()'), 'older settings must default to normal density');
assert.ok(backend.includes('Interface density must be tight, normal, or spacious.'), 'the native boundary must reject unknown densities');
assert.ok(settings.includes('aria-label="Interface density"') && settings.includes("['tight', 'normal', 'spacious']"), 'Appearance must expose a labelled three-stop density slider');
for (const token of ['--density-tabbar-height', '--density-sidebar-task-y', '--density-pane-header-y', '--density-detail-tabs-height']) {
  assert.ok(surface.includes(token), `workspace chrome must consume ${token}`);
}
assert.ok(surface.includes(':root[data-density="tight"]') && surface.includes(':root[data-density="spacious"]'), 'tight and spacious density variable sets must be defined');
assert.ok(surface.includes("root.dataset.density"), 'saved density must be applied to the document');
assert.ok(contract.includes('`interfaceDensity` is `tight`, `normal`, or `spacious`, defaulting to `normal`'), 'the shared appearance contract must document density migration');

console.log('interface density contract passed');
