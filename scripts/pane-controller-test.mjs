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
assert.ok(surface.includes('if (!node || !active) return'), 'inactive panes must release their DOM-local resize observer');
assert.doesNotMatch(surface, /display\s*:\s*contents/, 'PaneSurface must remain a real layout boundary');
assert.match(surface, /\.pane-surface\s*\{\s*display:flex/, 'PaneSurface owns its flex sizing boundary');
const localState = readFileSync(new URL('../src/lib/pane-local-state.svelte.ts', import.meta.url), 'utf8');
assert.match(localState, /class PaneLocalState/, 'each pane has a dedicated local state controller');
assert.match(localState, /selectionHistory/, 'tab selection history belongs to the pane controller');
assert.match(localState, /restore\(order/, 'tab ordering can be restored independently of the root layout');
const appSurface = readFileSync(new URL('../src/lib/components/AppSurface.svelte', import.meta.url), 'utf8');
assert.match(appSurface, /createPaneLocalState\(\)/, 'AppSurface delegates pane-local tab state to PaneLocalState');
assert.doesNotMatch(appSurface, /const tabSelectionHistory/, 'the root surface must not retain per-pane selection history');
assert.match(appSurface, /<RootSurfaceLifecycle start=\{startRootLifecycle\}/, 'only the root mounts bridge and workspace lifecycle listeners');
const rootLifecycle = readFileSync(new URL('../src/lib/components/RootSurfaceLifecycle.svelte', import.meta.url), 'utf8');
assert.match(rootLifecycle, /onMount\(\(\)\s*=>\s*start\(\)\)/, 'root lifecycle ownership is isolated from embedded pane mounts');
assert.ok(appSurface.includes('if (embedded) return;\n    const viewport = window.matchMedia'), 'only the root may create the global viewport and interface-scale listeners');
assert.ok(appSurface.includes('parentMobileSidebar={mobileSidebar}'), 'embedded panes inherit responsive state from their retained root');
assert.ok(appSurface.includes('use:rootMotion use:rootMobileViewport'), 'embedded panes must not install duplicate motion or viewport actions');
assert.match(appSurface, /<TaskTranscript/, 'task conversation rendering must be compiled separately from the root surface');
const transcript = readFileSync(new URL('../src/lib/components/TaskTranscript.svelte', import.meta.url), 'utf8');
assert.match(transcript, /<TranscriptVirtualList/, 'the transcript component retains virtualized conversation rendering');
assert.match(transcript, /<AnimatedTitle/, 'task headers keep animated title behavior after extraction');
assert.match(transcript, /<style>[\s\S]*\.conversation-head[\s\S]*\.message\.user/s, 'extracted transcript owns the scoped header and message styles it renders');
assert.match(transcript, /menuOpen[\s\S]*onMenuChange/, 'the child delegates menu state to the root keyboard-dismissal owner');
console.log('pane controller ownership routing assertions passed');
