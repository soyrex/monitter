import assert from 'node:assert/strict';
import { insertTab, normalizeTabOrder } from '../src/lib/tab-order.ts';

const task = { kind: 'task', id: 'task-a' };
const draft = { kind: 'draft', id: 'draft-a' };
const channel = { kind: 'channel', id: 'channel-a' };
const terminal = { kind: 'terminal', id: 'terminal-a' };
assert.deepEqual(normalizeTabOrder([channel, task], [task, draft, channel, terminal]), [channel, task, draft, terminal]);
assert.deepEqual(insertTab([task, draft, channel], channel, draft), [task, channel, draft]);
assert.deepEqual(insertTab([task, draft], terminal), [task, draft, terminal]);
console.log('tab order helpers: ok');
