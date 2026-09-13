import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const source=readFileSync(new URL('../src/lib/components/AppSurface.svelte',import.meta.url),'utf8');
assert.match(source,/const renderedTabCount = \$derived\(orderedTabs\(\)\.length/,'rendered pane tabs need a reactive count');
assert.match(source,/const useCompactTabPicker = \$derived\(\(mobileSidebar \|\| compactTabs\) && renderedTabCount > 1\)/,'compact picker must require at least two tabs');
assert.match(source,/class:compact-tabs=\{useCompactTabPicker\}/,'the responsive class must use the multi-tab guard');
assert.match(source,/if \(!useCompactTabPicker\) tabPickerOpen = false/,'an open picker must close when only one tab remains');

console.log('responsive tab picker contract passed');
