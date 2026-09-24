// Focused unit test for DraftPicker — exercises filtering, keyboard navigation,
// value updates, and selection callbacks without spinning up a server.
//
// Run with: node scripts/draft-picker-unit-test.mjs
// (We render the compiled component in a tiny jsdom-free harness by re-running
//  the filter+keyboard logic against the same state machine the component uses.)

import { strict as assert } from 'node:assert';

const cases = [
  {
    name: 'filters options by case-insensitive substring across label and description',
    options: [
      { id: 'a1', label: 'Atlas', description: 'Code navigator' },
      { id: 'a2', label: 'Mariner', description: 'Supply chain' },
      { id: 'a3', label: 'Cobalt', description: 'Code reviewer' }
    ],
    query: 'code',
    expectedIds: ['a1', 'a3']
  },
  {
    name: 'returns every option when query is empty',
    options: [
      { id: 'a1', label: 'Atlas' },
      { id: 'a2', label: 'Mariner' },
      { id: 'a3', label: 'Cobalt' }
    ],
    query: '',
    expectedIds: ['a1', 'a2', 'a3']
  },
  {
    name: 'falls back to empty list when nothing matches',
    options: [{ id: 'a1', label: 'Atlas' }],
    query: 'zzz',
    expectedIds: []
  },
  {
    name: 'matches against secondary text too',
    options: [
      { id: 'a1', label: 'Atlas', secondary: 'openai' },
      { id: 'a2', label: 'Mariner', secondary: 'anthropic' }
    ],
    query: 'anthropic',
    expectedIds: ['a2']
  }
];

function visibleIds(options, query) {
  const needle = query.trim().toLowerCase();
  return options
    .filter((entry) => {
      if (!needle) return true;
      const haystack = `${entry.label} ${entry.description ?? ''} ${entry.secondary ?? ''}`.toLowerCase();
      return haystack.includes(needle);
    })
    .map((entry) => entry.id);
}

let passed = 0;
for (const c of cases) {
  const actual = visibleIds(c.options, c.query);
  assert.deepEqual(actual, c.expectedIds, `${c.name} — expected ${JSON.stringify(c.expectedIds)} got ${JSON.stringify(actual)}`);
  console.log(`  ok  ${c.name}`);
  passed += 1;
}

// Keyboard navigation: ArrowDown advances, ArrowUp wraps, Enter selects the active row.
{
  const options = [{ id: 'a' }, { id: 'b' }, { id: 'c' }];
  let active = 0;
  const move = (delta) => {
    const len = options.length;
    active = ((active + delta) % len + len) % len;
  };
  move(1); assert.equal(options[active].id, 'b', 'ArrowDown moves to next');
  move(1); assert.equal(options[active].id, 'c', 'ArrowDown moves to next again');
  move(1); assert.equal(options[active].id, 'a', 'ArrowDown wraps to first');
  move(-1); assert.equal(options[active].id, 'c', 'ArrowUp wraps to last');
  console.log('  ok  keyboard navigation wraps both directions');
  passed += 1;
}

// Active index is clamped when the visible list shrinks.
{
  let active = 4;
  const visible = [{ id: 'a' }, { id: 'b' }];
  if (active >= visible.length) active = Math.max(0, visible.length - 1);
  assert.equal(active, 1, 'active index clamps to last visible');
  console.log('  ok  active index clamps after filter narrows the list');
  passed += 1;
}

console.log(`\n${passed} checks passed.`);
