import { readFileSync } from 'node:fs';
import assert from 'node:assert/strict';

const source = readFileSync(new URL('../src/lib/components/RunSummary.svelte', import.meta.url), 'utf8');
const style = source.slice(source.indexOf('<style>'));
const summary = style.match(/\.run-summary\{([^}]+)\}/)?.[1] ?? '';
const processes = style.match(/\.processes\{([^}]+)\}/)?.[1] ?? '';
assert.ok(!summary.includes('border-bottom'), 'Tracked processes must not leave an extra divider below the summary');
assert.ok(processes.includes('border-top:1px solid var(--line)'), 'The divider belongs above Tracked processes');
assert.ok(processes.includes('padding-top:'), 'Keep space between divider and heading');
console.log('Tracked processes divider is above its heading, not below the section.');
