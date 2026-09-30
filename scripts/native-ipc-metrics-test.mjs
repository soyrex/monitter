import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';

const source = fs.readFileSync(new URL('./native-ipc-metrics.js', import.meta.url), 'utf8');
const listeners = new Map(), raf = [], timers = new Set(), paragraphs = [];
let now = 0, appended = false, hidden = false;
const buttons = {};
const panel = { isConnected: false, style: {}, querySelector(selector) { return selector === '[data-results]' ? results : selector === '[data-status]' ? status : buttons[selector]; }, setAttribute() {}, append() {} };
const results = { replaceChildren(...items) { paragraphs.splice(0); paragraphs.push(...items); } };
const status = { textContent: '' };
for (const [key, selector] of [['start', '[data-start]'], ['stop', '[data-stop]'], ['reset', '[data-reset]']]) buttons[selector] = { disabled: key === 'stop', addEventListener(type, fn) { this[type] = fn; } };
const document = {
  body: null, hidden: false,
  createElement(tag) { return tag === 'aside' ? panel : { style: {}, textContent: '', setAttribute() {} }; },
  addEventListener(type, fn) { listeners.set(type, fn); },
  removeEventListener() {},
};
Object.defineProperty(document, 'hidden', { get: () => hidden });
const context = {
  document, Element: class Element {}, URL, window: {}, location: { href: 'https://fixture.test/app?x=1#chat' },
  history: { state: {}, replaceState(_state, _title, url) { context.replaced = String(url); } },
  performance: { now: () => now },
  requestAnimationFrame(fn) { raf.push(fn); return raf.length; },
  setTimeout(fn, ms) { const timer = { fn, at: now + ms }; timers.add(timer); return timer; },
  clearTimeout(timer) { timers.delete(timer); },
  setInterval() {}, clearInterval() {},
  PerformanceObserver: class { constructor(fn) { this.fn = fn; context.lastObserver = this; } observe() {} disconnect() {} },
  console,
};
function runDueTimers() { for (const timer of [...timers]) if (timer.at <= now) { timers.delete(timer); timer.fn(); } }
context.window = { PerformanceObserver: context.PerformanceObserver, matchMedia: () => ({ matches: false }), localStorage: { setItem() {} } };
vm.runInNewContext(source, context);
assert.equal(context.replaced, 'https://fixture.test/app?x=1&monitter-perf=1#chat');
document.body = { append(node) { node.isConnected = true; appended = true; } };
listeners.get('DOMContentLoaded')();
assert.equal(appended, true, 'head injection defers panel mount');

buttons['[data-start]'].click();
assert.equal(timers.size, 1);
const target = new context.Element();
target.matches = selector => selector === 'textarea'; target.closest = () => null;
listeners.get('input')({ type: 'input', target });
now = 5; raf.shift()(now); now = 8; raf.shift()(now); now = 12; raf.shift()(now); now = 20; raf.shift()(now); now = 300; runDueTimers();
assert.match(paragraphs[0]?.textContent ?? '', /composer-input: n=1\/1/);

// Reset must restart its generation, frame probe, and auto-stop clock.
buttons['[data-reset]'].click();
assert.equal(timers.size, 1, 'reset while active rearms auto-stop');
now = 30; raf.shift()(now); assert.ok(raf.length > 0, 'reset while active preserves frame sampling');
buttons['[data-stop]'].click();
const stale = raf.splice(0); for (const callback of stale) callback(31);
buttons['[data-start]'].click();
assert.equal(timers.size, 1, 'new run has one stop timer');

hidden = true; listeners.get('visibilitychange')(); now = 40; hidden = false; listeners.get('visibilitychange')();
const beforeHidden = paragraphs.length; now = 50; for (const callback of raf.splice(0)) callback(now);
assert.equal(paragraphs.length, beforeHidden, 'hidden epoch does not create a resumed frame-gap sample');

// Known performance phase names are accepted into separate buckets.
context.lastObserver.fn({ getEntries: () => [
  { name: 'monitter.chat-switch.open-to-mount', duration: 7 },
  { name: 'monitter.chat-switch.open-to-first-virtualizer-change', duration: 11 },
] });
now = 400; runDueTimers();
assert.ok(paragraphs.some(item => item.textContent.startsWith('chat-switch-open-to-mount: n=1/1')));
assert.ok(paragraphs.some(item => item.textContent.startsWith('chat-switch-open-to-first-virtualizer-change: n=1/1')));
console.log('native IPC metrics lifecycle test passed');
