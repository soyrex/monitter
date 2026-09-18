import assert from 'node:assert/strict';
import fs from 'node:fs';
import { compile } from 'svelte/compiler';
import * as $ from 'svelte/internal/client';

const source = fs.readFileSync(new URL('../src/lib/components/Markdown.svelte', import.meta.url), 'utf8');
const compiled = compile(source, { generate: 'client', dev: false }).js.code;
const declarationStart = compiled.indexOf('const sourceText');
const declarationEnd = compiled.indexOf('\n\tfunction makeImagesInteractive', declarationStart);
assert.ok(declarationStart >= 0 && declarationEnd > declarationStart, 'compiled Markdown declarations must remain extractable');
const optimizedDeclarations = compiled.slice(declarationStart, declarationEnd);
assert.match(optimizedDeclarations, /const sourceText = \$\.derived/, 'production must contain a primitive text barrier');
assert.match(optimizedDeclarations, /marked\.parse\(\$\.get\(sourceText\)/, 'Markdown parsing must consume the primitive barrier');
assert.match(optimizedDeclarations, /DOMPurify\.sanitize\(marked\.parse/, 'Markdown output must remain sanitized');

const baselineDeclarations = optimizedDeclarations
  .replace(/const sourceText = [\s\S]*?;\n\n\t/, '')
  .replace(/\$\.get\(sourceText\)/g, () => '$$props.text');

function run(declarations, values) {
  const message = $.mutable_source({ text: values[0] });
  const props = { get text() { return $.get(message).text; } };
  const counts = { parse: 0, sanitize: 0 };
  const marked = {
    parse(text) {
      counts.parse++;
      return `<p>${text}</p>`;
    },
  };
  const DOMPurify = {
    sanitize(html) {
      counts.sanitize++;
      return `sanitized:${html}`;
    },
  };
  const declarationsFactory = new Function('$', '$$props', 'marked', 'DOMPurify', 'preserveLineBreaks', 'allowedMarkdownUris', `${declarations}\nreturn { html };`);
  const { html } = declarationsFactory($, props, marked, DOMPurify, () => false, /.*/);
  let observed = $.get(html);
  const initial = { ...counts };
  for (const text of values.slice(1)) {
    $.set(message, { text });
    observed = $.get(html);
  }
  const result = { initial, final: { ...counts }, observed };
  return result;
}

const sameText = 'unchanged **history**';
const replacements = [sameText, ...Array(1000).fill(sameText)];
const baseline = run(baselineDeclarations, replacements);
const optimized = run(optimizedDeclarations, replacements);
assert.equal(baseline.final.parse - baseline.initial.parse, 1000, 'direct prop getter must reparse each replaced parent message');
assert.equal(baseline.final.sanitize - baseline.initial.sanitize, 1000, 'direct prop getter must resanitize each replaced parent message');
assert.equal(optimized.final.parse - optimized.initial.parse, 0, 'unchanged text must skip Markdown parsing');
assert.equal(optimized.final.sanitize - optimized.initial.sanitize, 0, 'unchanged text must skip sanitization');

const changed = run(optimizedDeclarations, [sameText, 'changed **content**']);
assert.equal(changed.final.parse - changed.initial.parse, 1, 'changed text must parse once');
assert.equal(changed.final.sanitize - changed.initial.sanitize, 1, 'changed text must sanitize once');
assert.equal(changed.observed, 'sanitized:<p>changed **content**</p>', 'rendered value must be sanitizer output');

console.log(JSON.stringify({
  baseline: {
    parse: baseline.final.parse - baseline.initial.parse,
    sanitize: baseline.final.sanitize - baseline.initial.sanitize,
  },
  optimized: {
    parse: optimized.final.parse - optimized.initial.parse,
    sanitize: optimized.final.sanitize - optimized.initial.sanitize,
  },
  changed: {
    parse: changed.final.parse - changed.initial.parse,
    sanitize: changed.final.sanitize - changed.initial.sanitize,
  },
  sanitizerOutputObserved: true,
}));
