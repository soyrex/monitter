import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const read = path => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8');
const app = read('src/lib/components/AppSurface.svelte');
const bridge = read('src/lib/bridge.ts');
const messagePane = read('src/lib/components/MessagePane.svelte');
const paneGrid = read('src/lib/components/PaneGrid.svelte');
const terminal = read('src/lib/terminal-runtime.ts');
const persistence = read('src/lib/workspace-persistence.ts');

function has(source, pattern, message) {
  assert.match(source, pattern, message);
}

// Inactive split panes must be able to remain retained without activating their
// expensive children. This checks the boundary rather than mounting a Tauri UI.
has(app, /<PaneGrid\s+\{layout\}/, 'the workspace must render through PaneGrid');
has(app, /<AppSurface\s+embedded=\{true\}[^\n]*\bactive=\{activePaneId===id/, 'embedded panes must receive activity state');
has(app, /<TerminalPane\s+sessionId=\{selectedTerminal\.id\}\s+active=/, 'terminal activation must be explicit');
has(paneGrid, /class:active=\{activePaneId===item\.id\}/, 'PaneGrid must retain active-pane identity');

// A poll that returns the same revision/object must not rebuild every derived
// list. Both the bridge request and the AppSurface application path are
// intentionally checked because either side regressing recreates the churn.
has(bridge, /cachedRevision\s*[=:]/, 'bridge must retain the snapshot revision');
has(bridge, /if\s*\(snapshotRequest\)\s*return snapshotRequest/, 'concurrent snapshot reads must coalesce');
has(app, /lastBridgeSnapshot\s*:\s*Snapshot\s*\|\s*null/, 'AppSurface must retain the unproxied snapshot identity');
has(app, /if\s*\(fromBridge\s*&&\s*next\s*===\s*lastBridgeSnapshot\)\s*return/, 'unchanged snapshots must skip visible reapplication');

// Full activity is paged at a fixed, cheap batch size. Chat panes must remain
// scroll containers so a future transcript window can bound DOM work without
// changing the surrounding layout contract.
has(app, /getTaskEvents\(taskId,\s*before,\s*100\)/, 'timeline reads must use a bounded page size');
has(messagePane, /class="messages"[^>]*\boverflow:\s*auto|\.messages\s*\{[^}]*overflow:\s*auto/, 'transcripts must use a scroll viewport');

// Activity-aware work must have a teardown path. These checks cover the two
// observer types and the interval used by the live terminal runtime.
has(messagePane, /const observer = new ResizeObserver/, 'MessagePane must observe layout changes');
has(messagePane, /const liveTextObserver = new MutationObserver/, 'MessagePane must observe streamed text changes');
has(messagePane, /observer\.disconnect\(\)[\s\S]*liveTextObserver\.disconnect\(\)[\s\S]*clearInterval\(interval\)/, 'MessagePane observers/timer must be cleaned up');
has(terminal, /function stopPolling\(runtime(?:\s*:\s*Runtime)?\)/, 'terminal polling must have an explicit stop path');
has(terminal, /runtime\.observer\?\.disconnect\(\)/, 'terminal resize observers must be disconnected');

// Persistence should not become an unbounded synchronous write on every
// pointer/resize event. Keep this assertion intentionally structural so it can
// accept either a named debounce helper or the existing bounded save boundary.
const persistenceCalls = app.match(/\bpersistWorkspace\(\)/g) ?? [];
assert.ok(persistenceCalls.length > 0, 'workspace changes must remain persisted');
has(app, /persistWorkspace\(\)[\s\S]{0,220}(?:setTimeout|requestAnimationFrame|workspaceTransition|pagehide|beforeunload)/,
  'workspace persistence must be scheduled or reserved for lifecycle boundaries');

// xterm is a heavyweight optional dependency. Keep it out of the root app
// surface and require runtime creation to remain lazy.
assert.doesNotMatch(app, /from ['"]@xterm\//, 'AppSurface must not import xterm directly');
has(terminal, /import\(['"]@xterm\/xterm['"]\)/, 'xterm must be dynamically imported');
has(terminal, /import\(['"]@xterm\/addon-fit['"]\)/, 'the fit addon must be dynamically imported');

// Guard the module boundary: the runtime is the only production module allowed
// to own polling, ResizeObserver setup, and xterm lifecycle.
for (const [name, source] of [['AppSurface', app], ['MessagePane', messagePane], ['workspace persistence', persistence]]) {
  assert.doesNotMatch(source, /new Terminal\s*\(/, `${name} must not construct xterm terminals`);
}

console.log('performance regression contracts passed');
