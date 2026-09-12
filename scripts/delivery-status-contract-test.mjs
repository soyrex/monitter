import { readFile } from 'node:fs/promises';
import { strict as assert } from 'node:assert';

const [surface, title] = await Promise.all([
  readFile(new URL('../src/lib/components/AppSurface.svelte', import.meta.url), 'utf8'),
  readFile(new URL('../src/lib/components/AnimatedTitle.svelte', import.meta.url), 'utf8'),
]);
assert.ok(surface.includes('<AnimatedTitle text="Sending" active={true} activeTooltip="Sending your message…"/>'), 'Only Sending should use the active sparkle title');
assert.ok(surface.includes('.delivery-status { margin-left:auto;'), 'Delivery status should consume remaining message-meta space on the right');
assert.ok(surface.includes("message.status === 'sent' ? 'Sent' : 'Not confirmed'"), 'Sent and Not confirmed must stay plain text');
assert.ok(title.includes("activeTooltip = 'Generating a title…'"), 'Existing autoname tooltip must remain the backward-compatible default');
assert.ok(title.includes('title={active ? activeTooltip : text}'), 'Active consumers must be able to supply a truthful tooltip');
console.log('delivery status contract: Sending sparkles with a sending tooltip and right alignment; other outcomes remain plain');
