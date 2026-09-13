import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { DEFAULT_BORDER_OPACITY, normalizeBorderOpacity } from '../src/lib/border-opacity.ts';

assert.equal(DEFAULT_BORDER_OPACITY,100);
assert.equal(normalizeBorderOpacity(-2),0);
assert.equal(normalizeBorderOpacity(38.7),39);
assert.equal(normalizeBorderOpacity(101),100);
assert.equal(normalizeBorderOpacity(Number.NaN),100);
const surface=readFileSync('src/lib/components/AppSurface.svelte','utf8');
assert.match(surface,/--line:\s*color-mix\(in srgb, var\(--line-colour\) var\(--border-opacity, 100%\), transparent\)/);
console.log('border opacity preference assertions passed');
