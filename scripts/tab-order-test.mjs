import assert from 'node:assert/strict';
import { insertTab, normalizeTabOrder } from '../src/lib/tab-order.ts';

const task = { kind: 'task', id: 'task-a' };
const draft = { kind: 'draft', id: 'draft-a' };
const channel = { kind: 'channel', id: 'channel-a' };
const terminal = { kind: 'terminal', id: 'terminal-a' };
const browser = { kind: 'browser', id: 'browser-a' };
assert.deepEqual(normalizeTabOrder([browser, channel, task], [task, draft, channel, terminal, browser]), [browser, channel, task, draft, terminal]);
assert.deepEqual(insertTab([task, draft, channel], channel, draft), [task, channel, draft]);
assert.deepEqual(insertTab([task, draft], terminal), [task, draft, terminal]);
assert.deepEqual(insertTab([task, browser, draft], browser, task), [browser, task, draft]);
console.log('tab order helpers: ok');
