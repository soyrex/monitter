import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { findPaneTabOwner, paneTabMatches } from '../src/lib/pane-controller.ts';

const tab = (kind, id, sourcePaneId = 'main') => ({ kind, id, sourcePaneId });
const panes = {
  left: { allTabs: () => [tab('task', 'one', 'left')] },
  right: { allTabs: () => [tab('terminal', 'shell', 'right')] },
};

assert.equal(findPaneTabOwner(['main', 'left', 'right'], 'right', [tab('channel', 'team')], panes, item => paneTabMatches(item, tab('task', 'one'))), 'left');
assert.equal(findPaneTabOwner(['main', 'left', 'right'], 'right', [tab('channel', 'team')], panes, item => paneTabMatches(item, tab('terminal', 'shell'))), 'right', 'active pane wins when multiple locations are possible');
assert.equal(findPaneTabOwner(['main', 'left', 'right'], 'left', [tab('channel', 'team')], panes, item => paneTabMatches(item, tab('channel', 'team'))), 'main');
assert.equal(findPaneTabOwner(['main', 'left'], 'main', [], panes, item => paneTabMatches(item, tab('task', 'missing'))), null);
const surface = readFileSync(new URL('../src/lib/components/PaneSurface.svelte', import.meta.url), 'utf8');
assert.ok(surface.includes('if (!observed || !active) return'), 'inactive panes must release their DOM-local resize observer');
assert.ok(surface.includes('display:contents'), 'the observer boundary must not change the existing workspace layout or scoped styles');
console.log('pane controller ownership routing assertions passed');
