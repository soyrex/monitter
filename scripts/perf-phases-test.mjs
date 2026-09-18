#!/usr/bin/env node
import assert from 'node:assert/strict';
import fs from 'node:fs';
import ts from 'typescript';

const source = fs.readFileSync(new URL('../src/lib/perf-phases.ts', import.meta.url), 'utf8');
const code = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
}).outputText;

let moduleRevision = 0;
async function load(search, { throwingMeasure = false } = {}) {
  const calls = [];
  const marks = new Map();
  const measures = new Map();
  let clock = 100;
  globalThis.window = { location: { search } };
  globalThis.performance = {
    now() { calls.push(['now']); return clock += 3; },
    clearMarks(name) { calls.push(['clearMarks', name]); marks.delete(name); },
    mark(name) { calls.push(['mark', name]); marks.set(name, { startTime: clock }); },
    getEntriesByName(name) { return marks.has(name) ? [{ startTime: marks.get(name).startTime }] : []; },
    clearMeasures(name) { calls.push(['clearMeasures', name]); measures.delete(name); },
    measure(name, ...args) {
      calls.push(['measure', name, ...args]);
      if (throwingMeasure) throw new Error('unsupported measure options');
      measures.set(name, args);
    },
  };
  // Fragments intentionally bust the data-URL module cache so geometryEnabled
  // is evaluated once per URL scenario, just as it is at page load.
  const mod = await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}#case-${moduleRevision++}`);
  return { mod, calls, marks, measures, advance(ms) { clock += ms; } };
}

const off = await load('');
assert.equal(off.mod.perfGeometryStart('row-measure'), undefined);
off.mod.perfMark('chat-open');
off.mod.perfMeasure('chat-open-to-mount', 'chat-open', 'chat-mount');
assert.equal(off.calls.length, 0, 'URL-off mode must make no Performance API calls');

const on = await load('?monitter-perf=1');
const finish = on.mod.perfGeometryStart('row-measure');
assert.equal(typeof finish, 'function');
finish();
const geometry = on.calls.find(call => call[0] === 'measure');
assert.equal(geometry[1], 'monitter.geometry.row-measure');
assert.equal(typeof geometry[2].start, 'number');
assert.equal(typeof geometry[2].end, 'number');
assert.ok(geometry[2].end >= geometry[2].start);

on.mod.perfMark('chat-open');
on.mod.perfMark('chat-mount');
on.mod.perfMeasure('chat-open-to-mount', 'chat-open', 'chat-mount');
assert.equal(on.marks.has('monitter:chat-open'), false, 'successful mount consumes chat-open mark');
const beforeStale = on.calls.length;
on.mod.perfMark('stale');
on.advance(5_001);
on.mod.perfMeasure('stale-measure', 'stale', 'end');
assert.ok(on.calls.length > beforeStale);
assert.equal(on.measures.has('monitter.stale-measure'), false, 'stale start must not be measured');

const throwing = await load('?monitter-perf=1', { throwingMeasure: true });
const safe = throwing.mod.perfGeometryStart('footer-measure');
assert.doesNotThrow(() => safe());
throwing.mod.perfMeasure('missing-start', 'missing', 'end');
assert.equal(throwing.measures.size, 0);

// Repeated names remain bounded to a single mark/measure after replacement.
const repeated = await load('?monitter-perf=1');
for (let i = 0; i < 10; i++) {
  repeated.mod.perfMark('repeat');
  repeated.mod.perfMark('repeat-end');
  repeated.mod.perfMeasure('repeat', 'repeat', 'repeat-end');
}
assert.equal(repeated.marks.size, 2);
assert.equal(repeated.measures.has('monitter.repeat'), true);
console.log('perf phase hooks passed');
