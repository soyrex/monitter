import { chromium, expect } from '@playwright/test';

const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18464';
const browser = await chromium.launch({ headless: true });

async function sample(mode) {
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.addInitScript(mode => localStorage.setItem('monitter.appearance.motion.v1', mode), mode);
  await page.addInitScript(() => {
    const qa = window.__MONITTER_QA__, snapshot = qa.snapshot(), now = Date.now();
    for (let index = 0; index < 200; index++) snapshot.tasks.push({
      id: `perf-${index}`, agentId: 'atlas', title: `Sidebar performance fixture ${index}`,
      nativeSessionId: null, status: index % 7 === 0 ? 'running' : 'idle', archived: false,
      createdAt: now - index, updatedAt: now - index, parentTaskId: null, channelId: null,
      projectId: null, hostId: 'local', cwd: '/tmp/monitter-ui-test', provider: 'codex', model: '', sandbox: 'read-only',
    });
    qa.setSnapshot(snapshot);
    window.__motionPerf = { frames: [], longTasks: [], running: true, last: 0 };
    const frame = now => { const perf = window.__motionPerf; if (perf.last) perf.frames.push(now - perf.last); perf.last = now; if (perf.running) requestAnimationFrame(frame); };
    requestAnimationFrame(frame);
    try { new PerformanceObserver(entries => window.__motionPerf.longTasks.push(...entries.getEntries().map(entry => entry.duration))).observe({ type: 'longtask', buffered: true }); } catch { /* unsupported */ }
  });
  try {
    await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 240_000 });
    await expect(page.getByRole('tab', { name: 'Agents view', exact: true })).toBeVisible({ timeout: 240_000 });
    await expect(page.locator('html')).toHaveAttribute('data-motion', mode);
    for (const view of ['Activity', 'Projects', 'Standard']) await page.getByRole('button', { name: `${view} view`, exact: true }).click();
    await page.waitForTimeout(180);
    await page.evaluate(() => { window.__motionPerf.frames = []; window.__motionPerf.longTasks = []; window.__motionPerf.last = 0; });
    for (let index = 0; index < 6; index++) for (const view of ['Activity', 'Projects', 'Standard']) await page.getByRole('button', { name: `${view} view`, exact: true }).click();
    await page.waitForTimeout(160);
    const ghostCount = await page.locator('[data-motion-ghost]').count();
    const result = await page.evaluate(() => {
      window.__motionPerf.running = false;
      const frames = [...window.__motionPerf.frames].sort((left, right) => left - right);
      const at = ratio => frames[Math.min(frames.length - 1, Math.floor(frames.length * ratio))] ?? null;
      return { frameCount: frames.length, p95Ms: at(.95), maxMs: frames.at(-1) ?? null, longTasks: window.__motionPerf.longTasks };
    });
    return { mode, ghostCount, errors, ...result };
  } finally { await page.close().catch(() => {}); }
}

try {
  const off = await sample('off');
  const subtle = await sample('subtle');
  console.log(JSON.stringify({ engine: 'chromium', taskCount: 200, transitionsPerMode: 18, off, subtle }, null, 2));
} finally { await browser.close(); }
