import { readFile } from 'node:fs/promises';
import { strict as assert } from 'node:assert';
import { paneRemovalDecision, paneTabCount, remapPromotedPaneId, remapQueuedPaneRemovals } from '../src/lib/pane-lifecycle.ts';

const empty = () => ({ openTaskIds: [], openDraftIds: [], openChannelIds: [], openTerminalIds: [], openEmptyIds: [], settingsOpen: false });
const terminal = id => ({ ...empty(), openTerminalIds: [id] });
const task = id => ({ ...empty(), openTaskIds: [id] });

// Exercise the actual lifecycle decision helper with a tiny deferred-removal
// harness. A collapse is requested only by a completed close/transfer.
function paneHarness({ panes, pending = false }) {
  const requests = new Set();
  const removed = [];
  const unreadable = new Set();
  return {
    request(id) { requests.add(id); },
    setPending(value) { pending = value; },
    setUnreadable(id, value) { if (value) unreadable.add(id); else unreadable.delete(id); },
    setPane(id, state) { panes[id] = state; },
    drain() {
      for (const id of [...requests]) {
        if (unreadable.has(id)) continue;
        const state = panes[id];
        const decision = state ? paneRemovalDecision(state, Object.keys(panes).length, pending) : 'keep';
        if (decision === 'keep') requests.delete(id);
        if (decision === 'remove') { requests.delete(id); removed.push(id); delete panes[id]; }
      }
    },
    removed,
    requests,
    panes,
  };
}

// A nested leaf can be briefly unbound while PaneGrid remounts after a close.
// Keep the request instead of mistaking an unreadable pane for a non-empty one.
{
  const h = paneHarness({ panes: { main: task('chat'), side: empty() } });
  h.request('side'); h.setUnreadable('side', true); h.drain();
  assert.deepEqual(h.removed, []);
  assert.ok(h.requests.has('side'));
  h.setUnreadable('side', false); h.drain();
  assert.deepEqual(h.removed, ['side']);
}

// The terminal closes its session before terminalBusy clears: removal is
// deferred, then happens after the busy flag clears.
{
  const h = paneHarness({ panes: { main: task('chat'), side: empty() }, pending: true });
  h.request('side'); h.drain();
  assert.deepEqual(h.removed, []);
  assert.ok(h.requests.has('side'));
  h.setPending(false); h.drain();
  assert.deepEqual(h.removed, ['side']);
}

// Unrelated pending work has the same retry behavior.
{
  const h = paneHarness({ panes: { main: task('chat'), side: empty() }, pending: true });
  h.request('side'); h.drain();
  h.setPending(false); h.drain();
  assert.deepEqual(h.removed, ['side']);
}

// A failed terminal close leaves its tab, so a queued collapse is discarded.
{
  const h = paneHarness({ panes: { main: task('chat'), side: terminal('term') } });
  h.request('side'); h.drain();
  assert.deepEqual(h.removed, []);
  assert.equal(h.requests.size, 0);
}

// Transfer requests removal only once acceptance is committed. A cancelled
// drop leaves the source tab and makes no request.
{
  const h = paneHarness({ panes: { main: task('source'), side: task('destination') } });
  h.drain(); // cancelled/failed transfer: no request
  assert.deepEqual(h.removed, []);
  h.setPane('main', empty()); // committed transfer removed the source tab
  h.request('main'); h.drain();
  assert.deepEqual(h.removed, ['main']);
}

// A fresh empty split has no close/transfer request and must remain. The sole
// remaining pane is also protected even when an explicit close is requested.
{
  const fresh = paneHarness({ panes: { main: task('chat'), side: empty() } });
  fresh.drain();
  assert.deepEqual(fresh.removed, []);
  const sole = paneHarness({ panes: { main: empty() } });
  sole.request('main'); sole.drain();
  assert.deepEqual(sole.removed, []);
}

assert.equal(paneTabCount({ ...empty(), openEmptyIds: ['new'], settingsOpen: true }), 2, 'Empty and Settings tabs keep a pane alive.');

// Removing main renames its promoted sibling to main. Expansion and queued
// removals must follow that ID instead of becoming stale references.
assert.equal(remapPromotedPaneId('side', 'main', 'side'), 'main');
assert.equal(remapPromotedPaneId('main', 'main', 'side'), null);
assert.deepEqual(remapQueuedPaneRemovals(['side', 'other', 'main', 'side'], 'main', 'side'), ['main', 'other']);

// AppSurface must use the helper plus its deferred root queue; inspect only the
// integration seams, while the cases above execute the actual decision logic.
const source = await readFile(new URL('../src/lib/components/AppSurface.svelte', import.meta.url), 'utf8');
for (const expected of [
  "import { paneRemovalDecision, paneTabCount, remapPromotedPaneId, remapQueuedPaneRemovals } from '$lib/pane-lifecycle';",
  'let pendingEmptyPaneIds = $state<string[]>([]);',
  'function queueEmptyPaneRemoval(id: string)',
  'await drainEmptyPaneRemovals();',
  "const decision = state ? paneRemovalDecision(state, paneIds(layout).length, layoutPending()) : 'keep';",
  'if (!state && paneIds(layout).includes(id)) continue;',
  "if (remaining && tabCount(remaining) === 0) await removeEmptyPane(tab.sourcePaneId);",
  'pendingEmptyPaneIds=remapQueuedPaneRemovals(pendingEmptyPaneIds,id,promoted);',
  'expandedPaneId=expandedPaneId===null?null:remapPromotedPaneId(expandedPaneId,id,promoted);',
  'function handleChildWorkspaceChange()',
  'onWorkspaceChange={handleChildWorkspaceChange}',
]) assert.ok(source.includes(expected), `Missing lifecycle integration: ${expected}`);
assert.ok(source.includes('closeSettings(collapse = true)') && source.includes("if(pane!=='settings'){if (collapse) collapseTablessPane();return;}"), 'Closing a non-selected Settings tab must still collapse a tabless pane.');
assert.ok(source.includes('if (collapse) collapseTablessPane();') && source.includes('finally{terminalBusy=false;}'), 'Terminal collapse is queued before busy clears and retried by the root queue.');
assert.ok(source.includes('Runtime terminal removal is also a tab close.') && source.includes('else openOverview();\n      // Runtime terminal removal is also a tab close. Do not retain a blank\n      // split merely because the session disappeared outside the tab button.\n      collapseTablessPane();'), 'A terminal session removed outside its tab Close control must also collapse a final-tab pane.');
assert.ok(source.includes('tabOrder=orderedTabs();\n      if(pane!==\'terminal\'||!selectedTerminalId||sessions[selectedTerminalId]) { collapseTablessPane(); return; }'), 'Runtime terminal cleanup must normalize stale tab order and collapse even when that terminal was not selected.');

console.log('pane lifecycle: deferred terminal/unrelated-busy collapse, failed-close safety, committed-transfer collapse, and fresh/sole-pane preservation pass.');
