import { readFile } from 'node:fs/promises';
import { strict as assert } from 'node:assert';

// Focused source contract for the tab strip. AppSurface is intentionally not
// booted: it owns native bridge setup and this test only guards tab chrome.
const source = await readFile(new URL('../src/lib/components/AppSurface.svelte', import.meta.url), 'utf8');
for (const expected of [
  'class="tab-entry task-tab" data-tab-kind={tab.kind}',
  '<MessageSquare size={13}/><span class="tab-shortcut"></span>',
  '<span class="tab-status" aria-label={`Status: ${task.status}`}><span class={`dot ${task.status}`}></span></span>',
  '.tab-entry:focus-within .tab-status { opacity:0; }',
  '.tab-picker-list .tab-status { position:absolute;',
  '.tab-picker-list .tab-entry:hover, .tab-picker-list .tab-entry:focus-within { background:var(--soft); }',
  '.tab-picker-list .tab { flex:1; width:0; min-width:0; max-width:none; min-height:44px; padding-right:8px; border:0; border-radius:5px; background:transparent; }',
  ".tabs.show-tab-index > .tab-picker-list > .tab-entry .tab-shortcut::after { content:counter(tab-index); }",
  '.tab-status { opacity:0; }',
]) assert.ok(source.includes(expected), `Missing task-tab UI contract: ${expected}`);

const taskMarkup = source.slice(source.indexOf("{:else if tab.kind === 'task'}"), source.indexOf("{:else if tab.kind === 'channel'}"));
assert.ok(taskMarkup.includes('class="tab-entry task-tab"'), 'The task branch itself must carry task-tab so its Cmd/Ctrl index overlays the chat icon');
assert.ok(taskMarkup.indexOf('class="tab-status"') < taskMarkup.indexOf('class="close-tab"'), 'Status must occupy the close slot before the hover Close control');
assert.ok(!taskMarkup.includes('<span class={`dot ${task.status}`}></span><span><AnimatedTitle'), 'Task status must not remain on the left of the title');
for (const kind of ['terminal', 'settings']) {
  const marker = `{:else if tab.kind === '${kind}'}`;
  const start = source.indexOf(marker, source.indexOf('<nav class="tabs tab-picker"'));
  assert.ok(start >= 0, `${kind} tab branch exists`);
  const branch = source.slice(start + marker.length).split('{/each}')[0].split("{:else if tab.kind ===")[0];
  assert.ok(branch.includes('class="tab-kind-icon"'), `${kind} needs a left icon slot`);
  if (kind === 'terminal') assert.ok(branch.includes('<SquareTerminal size={13}/>'), 'Terminal uses the selected boxed prompt icon');
  assert.ok(branch.includes('class="tab-shortcut"'), `${kind} needs a number overlay`);
  assert.ok(branch.indexOf('class="tab-kind-icon"') < branch.indexOf('class="close-tab"'), `${kind} Close remains after its icon/title`);
  assert.ok(source.includes(`:not([data-tab-kind='${kind}'])`), `${kind} must not get a number over its Close button`);
}
console.log('task/draft tab contract: chat icon, modifier index overlay, right status/hover close, touch close, and row-wide picker hover');
