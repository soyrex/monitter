import { readFileSync } from 'node:fs';
import assert from 'node:assert/strict';

const source = readFileSync('src/routes/share/+page.svelte', 'utf8');

assert.ok(source.includes("new URLSearchParams(url.hash.replace(/^#/, ''))"), 'fragment invite is supported');
assert.ok(source.includes("fragment.get('invite') || rawFragmentInvite") && source.includes("new URLSearchParams(url.search).get('invite')"), 'query invite remains a legacy fallback');
assert.match(source, /url\.searchParams\.delete\('invite'\)[\s\S]*window\.history\.replaceState/, 'invite token is removed with replaceState');
assert.match(source, /splitOperatorMessage\(message\.text\)/, 'shared operator attribution parser is used');
assert.match(source, /message\.senderAgentId.*snapshot\?\.agents\.find/, 'assistant messages resolve their agent name');
assert.match(source, /next\.tasks\.length === 1/, 'a sole shared chat opens automatically');
assert.match(source, /delivery: 'sending'/, 'send adds an optimistic local message');
assert.match(source, /delivery: 'uncertain'/, 'failed sends are visibly not confirmed');
assert.match(source, /if \(draft\.trim\(\) === text\) draft = ''/, 'new typing is preserved while the captured send is in flight');
assert.match(source, /baselineIds = new Set/, 'optimistic messages capture a pre-send baseline');
assert.match(source, /parsed\.text === item\.text && \(parsed\.name \?\? ''\) === item\.name/, 'reconciliation requires exact author and text');
assert.match(source, /item\.baselineIds\.has\(message\.id\)/, 'old identical messages cannot consume the optimistic entry');
assert.match(source, /message\.role === 'user' \? splitOperatorMessage/, 'only human user messages are parsed for operator tags');
assert.doesNotMatch(source, /refresh\(\)[\s\S]{0,180}scrollLatest\(\)/, 'refresh does not repeatedly force the conversation to the bottom');
console.log('Share visitor contract passed.');
