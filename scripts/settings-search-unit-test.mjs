import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const moduleURL = source => `data:text/javascript;base64,${Buffer.from(ts.transpile(source, { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 })).toString('base64')}`;
const navigationURL = moduleURL(readFileSync(new URL('../src/lib/settings-navigation.ts', import.meta.url), 'utf8'));
const searchSource = readFileSync(new URL('../src/lib/settings-search.ts', import.meta.url), 'utf8').replace("'./settings-navigation'", JSON.stringify(navigationURL));
const { searchSettings, SETTINGS_SEARCH_ENTRIES } = await import(moduleURL(searchSource));
for (const [query, id] of [['font size','interface-font-size'], ['transparncy','transparency'], ['reduce motion','reduce-motion'], ['guidance','guidance'], ['ifs','interface-font-size']]) {
  assert(searchSettings(query).some(entry => entry.id === id), `${query} should find ${id}`);
}
assert.equal(searchSettings('font size')[0].id, 'interface-font-size');
assert.deepEqual(searchSettings('  '), []);
assert.deepEqual(searchSettings('zxqvzzqq'), []);
const limited = SETTINGS_SEARCH_ENTRIES.filter(entry => entry.category === 'typography');
assert(searchSettings('font', limited).every(entry => entry.category === 'typography'));
assert.equal(searchSettings('guidance', limited).length, 0);
assert.equal(new Set(SETTINGS_SEARCH_ENTRIES.map(entry => entry.id)).size, SETTINGS_SEARCH_ENTRIES.length);
console.log('Preference search matching, typo, abbreviation and capability filtering checks passed.');
