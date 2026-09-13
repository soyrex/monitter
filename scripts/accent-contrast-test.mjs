import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { contrastForeground } from '../src/lib/accent-contrast.ts';

assert.equal(contrastForeground('#000000'),'#fff');
assert.equal(contrastForeground('#ffffff'),'#000');
assert.equal(contrastForeground('#3f9d6a'),'#000');
assert.equal(contrastForeground('#8755c7'),'#fff');
assert.equal(contrastForeground('#fff'),'#000');

const surface=readFileSync(new URL('../src/lib/components/AppSurface.svelte',import.meta.url),'utf8');
const mobile=readFileSync(new URL('../src/routes/mobile/+page.svelte',import.meta.url),'utf8');
assert.match(surface,/\.avatar \{[\s\S]*?color: var\(--on-accent\);[\s\S]*?background: var\(--accent\);/,'accent avatars must use the computed foreground');
assert.match(mobile,/--on-accent:\$\{contrastForeground\(accent\)\}/,'mobile must receive the computed foreground variable');
for(const declaration of [
  '.primary{background:var(--accent);color:var(--on-accent)}',
  '.view-tabs button.active{background:var(--accent);color:var(--on-accent)}',
  '.send{background:var(--accent);color:var(--on-accent)',
  '.bottom-nav button.active{color:var(--on-accent);background:var(--accent)}',
])assert.ok(mobile.includes(declaration),`${declaration.split('{')[0]} must use the computed foreground`);

console.log('accent contrast tests passed');
