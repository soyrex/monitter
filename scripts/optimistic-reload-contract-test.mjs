import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const source = readFileSync('src/lib/components/AppSurface.svelte', 'utf8');

assert.match(
  source,
  /if \(!embedded \|\| !parentSnapshot\) return;[\s\S]*?snapshot = next;[\s\S]*?untrack\(\(\) => reconcileOptimisticMessages\(next\)\)/,
  'an embedded pane must reconcile its local optimistic outbox when the root supplies a newer snapshot',
);

assert.match(
  source,
  /if \(optimisticMessages\.length && snapshot\) reconcileOptimisticMessages\(snapshot\)/,
  'a restored embedded-pane outbox must reconcile against its already-present parent snapshot',
);

const applySnapshot = source.slice(
  source.indexOf('function applySnapshot('),
  source.indexOf('function sameAttachments('),
);
const reconcile = applySnapshot.indexOf('reconcileOptimisticMessages(next)');
const unchangedReturn = applySnapshot.indexOf('next === lastBridgeSnapshot');
assert.ok(reconcile >= 0 && unchangedReturn >= 0 && reconcile < unchangedReturn,
  'optimistic messages must reconcile before an unchanged revision snapshot returns early');

console.log('Restored and revision-cached optimistic messages reconcile before rendering duplicate inputs.');
