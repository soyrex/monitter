import assert from 'node:assert/strict';
import fs from 'node:fs';
import { compile } from 'svelte/compiler';
import * as $ from 'svelte/internal/client';
import { flushSync } from 'svelte';

const source = fs.readFileSync(new URL('../src/lib/components/MessageMeta.svelte', import.meta.url), 'utf8');
const compiled = compile(source, { generate: 'client', dev: false }).js.code;
const declarationStart = compiled.indexOf('const timestampValue');
const declarationEnd = compiled.indexOf('var div =', declarationStart);
assert.ok(declarationStart >= 0 && declarationEnd > declarationStart, 'compiled declarations must remain extractable');
const optimizedDeclarations = compiled.slice(declarationStart, declarationEnd);
assert.match(optimizedDeclarations, /const timestampValue = \$\.derived/, 'production must contain primitive barrier');
const baselineDeclarations = optimizedDeclarations
  .replace(/const timestampValue = [\s\S]*?;\n\n\t/, '')
  .replace(/\$\.get\(timestampValue\)/g, () => '$$props.createdAt');
assert.doesNotMatch(baselineDeclarations, /timestampValue/);
assert.match(baselineDeclarations, /new Date\(\$\$props.createdAt\)/);

const RealDate = Date;
const counts = { constructed: 0, time: 0, timeString: 0, locale: 0 };
globalThis.Date = class CountingDate extends RealDate {
  constructor(...args) { super(...args); counts.constructed++; }
  getTime() { counts.time++; return super.getTime(); }
  toLocaleTimeString(...args) { counts.timeString++; return super.toLocaleTimeString(...args); }
  toLocaleString(...args) { counts.locale++; return super.toLocaleString(...args); }
};

function run(declarations, values) {
  const message = $.mutable_source({ createdAt: values[0] });
  const props = { get createdAt() { return $.get(message).createdAt; } };
  const declarationsFactory = new Function('$', '$$props', `${declarations}\nreturn { timestamp, validTimestamp, time };`);
  const { timestamp, validTimestamp, time } = declarationsFactory($, props);
  let observed = '', observedTitle = '';
  const stop = $.effect_root(() => $.render_effect(() => {
    observed = $.get(time);
    observedTitle = $.get(validTimestamp) ? `${$.get(timestamp).toISOString()}|${$.get(timestamp).toLocaleString()}` : '';
  }));
  flushSync();
  const initial = { ...counts };
  for (const createdAtValue of values.slice(1)) { $.set(message, { createdAt: createdAtValue }); flushSync(); }
  const result = { initial, final: { ...counts }, observed, observedTitle };
  stop();
  return result;
}

try {
const same = 1_725_000_000_000;
const baseline = run(baselineDeclarations, [same, ...Array(1000).fill(same)]);
const optimized = run(optimizedDeclarations, [same, ...Array(1000).fill(same)]);
assert.ok(baseline.final.constructed - baseline.initial.constructed > optimized.final.constructed - optimized.initial.constructed, 'primitive barrier should suppress unchanged Date reconstruction');
assert.equal(optimized.final.constructed - optimized.initial.constructed, 0, 'unchanged createdAt must not construct Dates');
assert.equal(optimized.final.timeString - optimized.initial.timeString, 0);
assert.equal(optimized.final.locale - optimized.initial.locale, 0);
assert.equal(baseline.final.timeString - baseline.initial.timeString, 1000);
assert.equal(baseline.final.locale - baseline.initial.locale, 1000);
assert.equal(optimized.observed, baseline.observed);
assert.equal(optimized.observedTitle, baseline.observedTitle);
const changed = run(optimizedDeclarations, [same, same + 1000]);
assert.equal(changed.final.constructed - changed.initial.constructed, 1, 'changed createdAt must update Date');
assert.notEqual(changed.observedTitle, optimized.observedTitle);
const invalid = run(optimizedDeclarations, [same, Number.NaN]);
assert.equal(invalid.observed, '', 'invalid date remains safely empty');
assert.equal(invalid.observedTitle, '');
console.log(JSON.stringify({ baselineDateBuilds: baseline.final.constructed - baseline.initial.constructed, optimizedDateBuilds: optimized.final.constructed - optimized.initial.constructed, changedDateBuilds: changed.final.constructed - changed.initial.constructed, invalidSafe: true }));
} finally {
  globalThis.Date = RealDate;
}
