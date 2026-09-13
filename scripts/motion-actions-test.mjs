import assert from 'node:assert/strict';
import { chromium, webkit } from '@playwright/test';

// Uses the already-running isolated preview only. This fixture imports the real
// Vite modules but never interacts with the Monitter bridge or app data.
const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18464/';
const engines = [chromium, webkit];
const failures = [];
const check = (condition, message) => { if (!condition) failures.push(message); };

const pause = (page, ms = 30) => page.waitForTimeout(ms);

for (const engine of engines) {
  const browser = await engine.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 900, height: 700 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  try {
    await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 60_000 });
    // Let the preview's own hydration/focus work settle before the standalone fixture.
    await pause(page, 100);
    await page.evaluate(async () => {
      window.__motionTestCalls = [];
      const originalAnimate = Element.prototype.animate;
      Element.prototype.animate = function (frames, options) {
        if (this.hasAttribute('data-motion-test')) window.__motionTestCalls.push(this.getAttribute('data-motion-test'));
        return originalAnimate.call(this, frames, options);
      };
      window.__motionTest = {
        motion: await import('/src/lib/motion.ts'),
        navigation: await import('/src/lib/navigation-motion.ts'),
        tabStrip: await import('/src/lib/tab-strip-fade.ts'),
      };
    });

    // Policy: manual Off cancels active WAAPI, and OS reduced motion is a hard gate.
    const policy = await page.evaluate(() => {
      const { motion } = window.__motionTest;
      document.body.insertAdjacentHTML('beforeend', '<div id="motion-policy-root"><div data-motion-test="policy"></div></div>');
      const root = document.querySelector('#motion-policy-root');
      const target = root.firstElementChild;
      const action = motion.initMotion(root);
      motion.setMotionPreference('subtle');
      const animation = motion.animateMotion(target, [{ opacity: 0 }, { opacity: 1 }], { duration: 1_000 });
      motion.setMotionPreference('off');
      return { created: !!animation, state: animation?.playState, enabled: motion.motionEnabled(), mode: document.documentElement.dataset.motion, calls: window.__motionTestCalls.length, action };
    });
    check(policy.created, `${engine.name()}: subtle policy should start WAAPI`);
    check(policy.state === 'idle', `${engine.name()}: Off should cancel a running animation`);
    check(!policy.enabled && policy.mode === 'off', `${engine.name()}: Off should publish the off policy`);
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await pause(page);
    const reduced = await page.evaluate(() => {
      const { motion } = window.__motionTest;
      motion.setMotionPreference('subtle');
      const target = document.querySelector('[data-motion-test="policy"]');
      return { enabled: motion.motionEnabled(), mode: document.documentElement.dataset.motion, animation: motion.animateMotion(target, [{ opacity: 0 }, { opacity: 1 }], { duration: 100 }) };
    });
    check(!reduced.enabled && reduced.mode === 'off' && reduced.animation === null, `${engine.name()}: OS reduced motion must override Subtle`);
    await page.emulateMedia({ reducedMotion: 'no-preference' });
    await pause(page);

    // Retained views: no remount/duplicate nodes, focus survives a keyed entrance,
    // and hidden or disabled views do not produce animation work.
    const view = await page.evaluate(async () => {
      const { motion } = window.__motionTest;
      motion.setMotionPreference('subtle');
      document.body.insertAdjacentHTML('beforeend', '<section id="motion-view-root"><div data-motion-test="view"><button>Stay focused</button></div></section>');
      const root = document.querySelector('#motion-view-root');
      const node = root.firstElementChild;
      const button = node.querySelector('button');
      button.focus();
      const action = motion.motionView(node, { key: 'first', duration: 400 });
      const before = root.children.length;
      action.update({ key: 'second', duration: 400 });
      await new Promise(resolve => requestAnimationFrame(resolve));
      const focused = document.activeElement === button;
      const after = root.children.length;
      const afterVisible = window.__motionTestCalls.filter(name => name === 'view').length;
      node.style.display = 'none';
      const hiddenRects = node.getClientRects().length;
      action.update({ key: 'third', duration: 400 });
      await new Promise(resolve => requestAnimationFrame(resolve));
      const hiddenCalls = window.__motionTestCalls.filter(name => name === 'view').length;
      action.update({ key: 'fourth', enabled: false, duration: 400 });
      await new Promise(resolve => requestAnimationFrame(resolve));
      const disabledCalls = window.__motionTestCalls.filter(name => name === 'view').length;
      action.destroy();
      return { before, after, focused, afterVisible, hiddenRects, hiddenCalls, disabledCalls };
    });
    check(view.before === 1 && view.after === 1 && view.focused, `${engine.name()}: motionView should retain its one focused node (${JSON.stringify(view)})`);
    check(view.hiddenRects === 0 && view.hiddenCalls === view.afterVisible && view.disabledCalls === view.afterVisible,
      `${engine.name()}: motionView should skip hidden/disabled keyed entries (calls ${view.afterVisible}/${view.hiddenCalls}/${view.disabledCalls})`);

    // Outgoing copies are inert and stripped of duplicate identifiers, replace
    // rather than stack on reversal, and clean up after their short exit.
    const outgoing = await page.evaluate(async () => {
      const { motion, navigation } = window.__motionTest;
      motion.setMotionPreference('subtle');
      document.body.insertAdjacentHTML('beforeend', '<div id="out-parent" style="position:relative"><div data-motion-test="outgoing" id="live-id" role="status" aria-live="polite"><input id="nested-id" autofocus><span>old view</span></div></div>');
      const live = document.querySelector('[data-motion-test="outgoing"]');
      navigation.outgoingVisual(live, -4, 0, 80);
      const first = document.querySelector('[data-motion-ghost]');
      const safe = !!first && first.getAttribute('aria-hidden') === 'true' && first.inert && !first.querySelector('[id], [role], [aria-live], [autofocus]');
      navigation.outgoingVisual(live, 4, 0, 80);
      const countAfterReverse = document.querySelectorAll('[data-motion-ghost]').length;
      await new Promise(resolve => setTimeout(resolve, 140));
      const cleaned = document.querySelectorAll('[data-motion-ghost]').length;
      const oversizedParent = document.createElement('div');
      oversizedParent.style.position = 'relative';
      const oversized = document.createElement('div');
      oversized.dataset.motionTest = 'oversized-outgoing';
      for (let index = 0; index < 513; index += 1) oversized.append(document.createElement('span'));
      oversizedParent.append(oversized);
      document.body.append(oversizedParent);
      const oversizedBefore = { parentChildren: oversizedParent.children.length, descendants: oversized.querySelectorAll('*').length };
      navigation.outgoingVisual(oversized, 4, 0, 80);
      const oversizedSkipped = document.querySelectorAll('[data-motion-ghost]').length === 0
        && oversizedParent.children.length === oversizedBefore.parentChildren
        && oversized.querySelectorAll('*').length === oversizedBefore.descendants;
      motion.setMotionPreference('off');
      navigation.outgoingVisual(live, 4, 0, 80);
      const offCount = document.querySelectorAll('[data-motion-ghost]').length;
      return { safe, countAfterReverse, cleaned, oversizedSkipped, offCount };
    });
    check(outgoing.safe && outgoing.countAfterReverse === 1 && outgoing.cleaned === 0 && outgoing.oversizedSkipped && outgoing.offCount === 0,
      `${engine.name()}: outgoingVisual must be inert, deduplicated and cleaned`);

    // Arrival observer ignores loaded/history batches and ordinary replies; only
    // a single optimistic message or a live replacement for thinking enters.
    const arrival = await page.evaluate(async () => {
      const { motion, navigation } = window.__motionTest;
      motion.setMotionPreference('subtle');
      document.body.insertAdjacentHTML('beforeend', '<div class="messages" style="height:150px;overflow:auto"><div id="message-host"></div></div>');
      const host = document.querySelector('#message-host');
      const action = navigation.messageArrival(host);
      const calls = () => window.__motionTestCalls.filter(name => name === 'message').length;
      const add = (className, live = false) => { const item = document.createElement('div'); item.className = `message ${className}`; item.dataset.motionTest = 'message'; if (live) item.dataset.liveEntry = 'true'; item.textContent = 'message'; host.append(item); };
      add('history'); add('history');
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      const history = calls();
      add('optimistic-message');
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      const optimistic = calls();
      host.replaceChildren();
      const pending = document.createElement('div'); pending.className = 'reasoning-pending'; host.append(pending);
      const reply = document.createElement('div'); reply.className = 'message'; reply.dataset.motionTest = 'message'; reply.dataset.liveEntry = 'true'; reply.textContent = 'live reply';
      pending.replaceWith(reply);
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      const replacement = calls();
      action.destroy();
      return { history, optimistic, replacement };
    });
    check(arrival.history === 0, `${engine.name()}: history batch must not replay entries`);
    check(arrival.optimistic === 1, `${engine.name()}: one optimistic send should enter once`);
    check(arrival.replacement === 2, `${engine.name()}: live thinking replacement should enter once`);

    // Edge fades/reveal operate only within the strip; this also catches the
    // module import alias and ResizeObserver path in both engines.
    const tabs = await page.evaluate(async () => {
      const { motion, tabStrip } = window.__motionTest;
      motion.setMotionPreference('subtle');
      document.body.insertAdjacentHTML('beforeend', '<div id="tabs" style="width:120px;overflow:auto;white-space:nowrap"><span class="tab-entry" style="display:inline-block;width:100px">one</span><span class="tab-entry active" style="display:inline-block;width:100px">two</span></div>');
      const strip = document.querySelector('#tabs');
      const action = tabStrip.tabStripFade(strip);
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      const result = { scrolled: strip.scrollLeft > 0, left: strip.style.getPropertyValue('--tab-fade-left'), right: strip.style.getPropertyValue('--tab-fade-right') };
      action.destroy();
      return result;
    });
    check(tabs.scrolled && tabs.left === '20px' && tabs.right === '0px', `${engine.name()}: active tab should reveal inside its own strip`);
    check(errors.length === 0, `${engine.name()}: fixture browser errors: ${errors.join('; ')}`);
    console.log(`${engine.name()}: motion actions completed`);
  } finally {
    await browser.close();
  }
}

assert.deepEqual(failures, [], `Motion action failures:\n${failures.map(item => `- ${item}`).join('\n')}`);
