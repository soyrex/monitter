import assert from 'node:assert/strict';
import { get } from 'svelte/store';

const values = new Map();
globalThis.localStorage = {
  getItem: key => values.has(key) ? values.get(key) : null,
  setItem: (key, value) => values.set(key, String(value)),
};

const scale = await import('../src/lib/interface-scale.ts');

assert.equal(scale.normalizeInterfaceScale(79), 80);
assert.equal(scale.normalizeInterfaceScale(203), 200);
assert.equal(scale.normalizeInterfaceScale(127), 125);
assert.equal(scale.seedViewerInterfaceScale('desktop', 130), 130);
assert.equal(scale.seedViewerInterfaceScale('mobile', 95), 95);
assert.equal(values.get(scale.interfaceScaleStorageKey('desktop')), '130');
assert.equal(values.get(scale.interfaceScaleStorageKey('mobile')), '95');

scale.setViewerInterfaceScale('desktop', 175);
assert.equal(get(scale.interfaceScaleStore('desktop')), 175);
assert.equal(get(scale.interfaceScaleStore('mobile')), 95);

// A later connection snapshot must not replace a viewer preference that was already seeded.
assert.equal(scale.seedViewerInterfaceScale('desktop', 80), 175);
assert.equal(scale.seedViewerInterfaceScale('mobile', 200), 95);

console.log('desktop and mobile interface scales persist independently of connection settings');
