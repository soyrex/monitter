import assert from 'node:assert/strict';
import { observeGit, resetGitDetectorForTest } from '../src/lib/git-detector.ts';

const tick = () => new Promise(resolve => setTimeout(resolve, 0));
let probes = 0, waits = 0, resolveProbe, resolveWait;
const bridge = {
  getTaskGitStatus: async () => {
    probes++;
    return new Promise(resolve => { resolveProbe = resolve; });
  },
  waitForTaskGitMarker: async () => {
    waits++;
    return new Promise(resolve => { resolveWait = resolve; });
  },
};

resetGitDetectorForTest();
const first = [], second = [];
const stopFirst = observeGit(bridge, 'a', 'local\u001ffolder', status => first.push(status.repository));
const stopSecond = observeGit(bridge, 'b', 'local\u001ffolder', status => second.push(status.repository));
assert.equal(probes, 1, 'same folder has one in-flight probe');
resolveProbe({ repository: false });
await tick();
assert.deepEqual(first, [false]); assert.deepEqual(second, [false]); assert.equal(waits, 1);
resolveWait('timeout'); await tick();
assert.equal(probes, 1, 'timeout re-arms marker wait without Git'); assert.equal(waits, 2);
stopFirst(); stopSecond(); resolveWait('timeout'); await tick();
assert.equal(waits, 2, 'closed panes do not keep marker waits alive');

resetGitDetectorForTest(); probes = 0; waits = 0;
const changes = [];
observeGit(bridge, 'c', 'local\u001fnew-folder', status => changes.push(status.repository));
resolveProbe({ repository: false }); await tick(); resolveWait('found'); await tick();
assert.equal(probes, 2, 'a created marker triggers exactly one fresh Git check');
resolveProbe({ repository: true, root: '/repo', branch: 'main', files: [], truncated: false }); await tick();
assert.deepEqual(changes, [false, true]);
console.log('git detector behaviors passed');
