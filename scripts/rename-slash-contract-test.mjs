import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const app = readFileSync('src/lib/components/AppSurface.svelte', 'utf8');
const slashRegistration = app.match(/monitterSlash\("autoname",\s*"([^"]+)"/);
assert.equal(slashRegistration?.[1], '/rename', 'the naming slash command should be offered as /rename');
assert.ok(app.includes('item.id === "autoname") await autonameCurrentPane()'), 'the renamed slash command should keep its existing naming action');
assert.ok(app.includes('keywords:"/rename title"'), 'Controls search should use the new slash name');
assert.ok(!app.includes('"/autoname"'), 'the old slash name should no longer be offered');

console.log('/rename slash registration and existing naming action passed.');
