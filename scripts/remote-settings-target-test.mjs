import assert from 'node:assert/strict';
import { get } from 'svelte/store';
import { remoteControlTarget, setRemoteControlTarget } from '../src/lib/workspace-panels.ts';

// Test-only DOM stand-ins; selection never needs to access production app state.
const left = { isConnected: true }, right = { isConnected: true };
const releaseLeft = setRemoteControlTarget(left, true);
const releaseRight = setRemoteControlTarget(right, false);
assert.equal(get(remoteControlTarget), left, 'A later inactive pane must not steal the controls.');
releaseLeft();
assert.equal(get(remoteControlTarget), right, 'Closing the focused pane restores another visible target.');
const releaseActiveLeft = setRemoteControlTarget(left, true);
releaseRight();
assert.equal(get(remoteControlTarget), left, 'An old cleanup must not detach a newer target.');
const releaseActiveRight = setRemoteControlTarget(right, true);
assert.equal(get(remoteControlTarget), right);
releaseActiveRight();
assert.equal(get(remoteControlTarget), left);
right.isConnected = false;
const releaseDetached = setRemoteControlTarget(right, true);
assert.equal(get(remoteControlTarget), left, 'Detached targets cannot receive the live panel.');
releaseDetached();
releaseActiveLeft();
assert.equal(get(remoteControlTarget), null, 'No visible Settings target means no floating panel.');
console.log('Remote Settings target ownership passed.');
