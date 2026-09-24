import assert from 'node:assert/strict';
import { chromium, expect } from '@playwright/test';

const browser = await chromium.launch({ headless: true, executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' });
try {
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.addInitScript(() => {
    const qa = window.__MONITTER_QA__, snapshot = qa.snapshot(), now = Date.now();
    snapshot.tasks = [{ id: 'long-stream', agentId: 'atlas', title: 'Long stream geometry', nativeSessionId: null, status: 'running', archived: false, createdAt: now, updatedAt: now, parentTaskId: null, channelId: null, projectId: null, hostId: 'local', cwd: '/tmp/monitter-ui-test', provider: 'codex', model: '', sandbox: 'read-only' }];
    snapshot.messages = [
      { id: 'long-user', taskId: 'long-stream', role: 'user', text: `Synthetic long request: ${'A'.repeat(4096)}`, createdAt: now - 1000 },
      { id: 'live-reply', taskId: 'long-stream', role: 'assistant', text: 'Synthetic response starts here.', streamStatus: 'streaming', createdAt: now },
    ];
    qa.setSnapshot(snapshot);
  });
  await page.goto(process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18420/monitter-app-ui/', { waitUntil: 'domcontentloaded', timeout: 120_000 });
  await page.locator('[data-task-id="long-stream"] .task-select').click();
  await expect(page.locator('[data-stream-text]')).toBeVisible();
  const metrics = await page.evaluate(async () => {
    const qa = window.__MONITTER_QA__;
    const viewport = document.querySelector('.messages');
    const pane = document.querySelector('.pane-leaf');
    const request = document.querySelector('.sticky-user-request');
    const live = document.querySelector('[data-stream-text]');
    const samples = [];
    let tick = 0;
    const interval = window.setInterval(() => {
      const state = qa.snapshot();
      state.messages.find(message => message.id === 'live-reply').text += `\nSynthetic streamed line ${++tick}: ${'B'.repeat(200)}`;
      qa.setSnapshot(state);
    }, 75);
    try {
      const end = performance.now() + 2200;
      while (performance.now() < end) {
        await new Promise(resolve => requestAnimationFrame(resolve));
        samples.push({
          gap: Math.round(viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop),
          viewportHeight: viewport.clientHeight,
          paneWidth: Math.round(pane.getBoundingClientRect().width),
          requestHeight: Math.round(request?.getBoundingClientRect().height ?? -1),
          liveMounted: document.querySelector('[data-stream-text]') === live,
        });
      }
    } finally { window.clearInterval(interval); }
    return { ticks: tick, maxGap: Math.max(...samples.map(sample => sample.gap)), widths: [...new Set(samples.map(sample => sample.paneWidth))], requestHeights: [...new Set(samples.map(sample => sample.requestHeight))], liveRemountFrames: samples.filter(sample => !sample.liveMounted).length, sampleCount: samples.length };
  });
  assert.ok(metrics.ticks >= 15, `too few stream updates: ${metrics.ticks}`);
  assert.ok(metrics.maxGap <= 3, `stream must stay at bottom; max gap ${metrics.maxGap}px`);
  assert.equal(metrics.widths.length, 1, `pane width changed: ${metrics.widths}`);
  assert.equal(metrics.requestHeights.length, 1, `request height changed during streaming: ${metrics.requestHeights}`);
  assert.equal(metrics.liveRemountFrames, 0, 'streaming text remounted');
  const expand = page.getByRole('button', { name: 'Expand current request' });
  await expect(expand).toBeVisible();
  await expand.click();
  await expect(page.getByRole('button', { name: 'Collapse current request' })).toBeVisible();
  console.log(JSON.stringify({ metrics, errors }));
  assert.deepEqual(errors, []);
} finally { await browser.close(); }
