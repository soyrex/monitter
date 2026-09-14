import assert from 'node:assert/strict';
import { createWorkspaceSaveScheduler } from '../src/lib/workspace-save-scheduler.ts';

const waits = ms => new Promise(resolve => setTimeout(resolve, ms));
let saves = 0;
const scheduler = createWorkspaceSaveScheduler(() => { saves++; return true; }, 10);

scheduler.schedule(); scheduler.schedule(); scheduler.schedule();
assert.equal(scheduler.pending, true, 'repeated changes must share one pending write');
assert.equal(saves, 0);
await waits(25);
assert.equal(saves, 1, 'coalesced changes must write once after the delay');
assert.equal(scheduler.pending, false);

scheduler.schedule();
assert.equal(scheduler.flush(), true, 'flush must return the save result');
assert.equal(saves, 2);
assert.equal(scheduler.pending, false);

scheduler.schedule(); scheduler.cancel(); await waits(25);
assert.equal(saves, 2, 'cancel must prevent a queued write');
console.log('workspace save scheduler behaviors passed');
