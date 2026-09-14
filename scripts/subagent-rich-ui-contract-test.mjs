import { readFile } from 'node:fs/promises';
import { strict as assert } from 'node:assert';

const [surface, activity] = await Promise.all([
  readFile(new URL('../src/lib/components/AppSurface.svelte', import.meta.url), 'utf8'),
  readFile(new URL('../src/lib/components/SubagentActivity.svelte', import.meta.url), 'utf8'),
]);

assert.match(surface, /detailTab:'run'\|'git'\|'timeline'\|'approvals'\|'subagents'/, 'pane state persists the Subagents tab');
assert.ok(surface.includes('Subagents <span>{taskSubagents.length}</span>'), 'tab shows the number of subagents');
assert.ok(surface.includes('item.value.kind===\'collaboration\''), 'chat stream recognizes durable collaboration events');
assert.ok(surface.includes('event.kind === "collaboration"'), 'durable collaboration events are included in the chat stream');
assert.ok(surface.includes('No subagents have run from this chat.'), 'empty state stays explicit');
for (const label of ['Spawned ${name}', '${name} started', '${name} finished', '${name} stopped', 'Messaged ${name}', 'Steered ${name}']) {
  assert.ok(activity.includes(label), `lifecycle label is present: ${label}`);
}
assert.ok(activity.includes('terminalEvent ? collaboration.error || collaboration.result || collaboration.text : collaboration.text'), 'terminal cards prefer the real result or error while lifecycle cards retain their request');
console.log('Rich subagent stream lifecycle and pane sidebar contract passed.');
