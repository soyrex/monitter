import { webkit, expect as baseExpect } from '@playwright/test';
import { mkdirSync, readFileSync } from 'node:fs';
import { spawn } from 'node:child_process';

const expect = baseExpect.configure({ timeout: 60000 });
const children = [];
const start = (command, args, env, pattern) => new Promise((resolve, reject) => {
  const child = spawn(command, args, { env: { ...process.env, ...env }, stdio: ['ignore', 'pipe', 'pipe'] });
  children.push(child);
  let output = '';
  const timer = setTimeout(() => reject(Error(`Timed out starting ${command}`)), 60000);
  const receive = data => { output += data.toString(); const match = output.match(pattern); if (match) { clearTimeout(timer); resolve(match[1]); } };
  child.stdout.on('data', receive); child.stderr.on('data', receive);
  child.once('error', error => { clearTimeout(timer); reject(error); });
  child.once('exit', code => { clearTimeout(timer); reject(Error(`${command} exited ${code}: ${output.slice(-1000)}`)); });
});

const taskId = '11111111-1111-4111-8111-111111111111';
const now = Date.now();
const snapshot = (scale = 100) => ({
  hosts: [{ id: 'local', name: 'This Mac', kind: 'local', address: '', user: '', port: 0, identityFile: '', defaultCwd: '/tmp/monitter-ui-test', codexPath: 'codex', claudePath: '', opencodePath: '', hermesPath: '' }],
  agents: [{ id: 'atlas', avatar: null, name: 'Atlas', description: 'QA agent', instructions: '', provider: 'codex', model: '', hostId: 'local', cwd: '/tmp/monitter-ui-test', color: '#397e61', sandbox: 'read-only', expertise: [], responsibilities: [], skills: [], collaborationEnabled: true }],
  tasks: [{ id: taskId, agentId: 'atlas', title: 'Compact metadata chat', nativeSessionId: 'qa-native', status: 'completed', archived: false, createdAt: now - 1000, updatedAt: now, parentTaskId: null, channelId: null, projectId: null, hostId: 'local', cwd: '/tmp/monitter-ui-test', provider: 'codex', model: '', sandbox: 'read-only' }],
  messages: [
    { id: 'user-message', taskId, role: 'user', text: 'A user message for layout checks.', createdAt: now - 800 },
    { id: 'assistant-message', taskId, role: 'assistant', text: 'An assistant message for layout checks.', createdAt: now - 500 },
  ],
  events: [], channels: [], projects: [], collaborations: [], queuedMessages: [],
  settings: { accent: '#3f9d6a', theme: 'light', interfaceFontSize: 14 * scale / 100, chatFontSize: 13 * scale / 100, interfaceScale: scale, showToolActivity: true, showReasoningSummaries: true, sendWithEnter: false, sidebarView: 'standard' },
});

async function assertHeaders(page, label, bodySelector) {
  const meta = page.locator('.message-meta').first();
  await expect(meta).toBeVisible();
  await expect(meta.locator('.message-author')).toHaveCount(1);
  const author = meta.locator('.message-author');
  const time = meta.locator('time');
  await expect(time).toHaveCount(1);
  const datetime = await time.getAttribute('datetime');
  expect(datetime, `${label}: datetime`).toMatch(/^\d{4}-\d{2}-\d{2}T/);
  const formattedTime = (await time.textContent())?.trim();
  expect(formattedTime, `${label}: formatted time`).toMatch(/^\d{1,2}:\d{2}/);
  const body = page.locator(bodySelector).first();
  await expect(body).toBeVisible();
  const metrics = await page.evaluate((bodySelector) => {
    const meta = document.querySelector('.message-meta'), author = meta?.querySelector('.message-author'), time = meta?.querySelector('time'), body = document.querySelector(bodySelector);
    if (!meta || !author || !time || !body) return null;
    const style = (el) => getComputedStyle(el);
    const a = style(author), t = style(time), m = meta.getBoundingClientRect(), b = body.getBoundingClientRect();
    return { authorSize: parseFloat(a.fontSize), timeSize: parseFloat(t.fontSize), bodySize: parseFloat(style(body).fontSize), lineHeight: parseFloat(a.lineHeight), gap: b.top - m.bottom, display: style(meta).display, authorTop: author.getBoundingClientRect().top, timeTop: time.getBoundingClientRect().top };
  }, bodySelector);
  expect(metrics, `${label}: metadata metrics`).not.toBeNull();
  expect(metrics.display, `${label}: inline metadata`).toMatch(/inline|flex/);
  expect(metrics.timeSize, `${label}: time smaller than name`).toBeLessThan(metrics.authorSize);
  expect(metrics.bodySize, `${label}: body larger than name`).toBeGreaterThan(metrics.authorSize);
  expect(metrics.lineHeight, `${label}: compact line-height`).toBeCloseTo(metrics.authorSize * 1.3, 1);
  expect(metrics.gap, `${label}: metadata/body gap`).toBeGreaterThanOrEqual(0);
  expect(metrics.gap, `${label}: metadata/body gap`).toBeLessThanOrEqual(4.5);
  expect(Math.abs(metrics.authorTop - metrics.timeTop), `${label}: author/time same line`).toBeLessThan(2);
  return metrics;
}

async function desktopCheck(url, browser) {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.goto(url, { timeout: 90000 });
  await expect(page.locator('.app-shell').first()).toBeVisible();
  await page.evaluate(value => window.__MONITTER_QA__.setSnapshot(value), snapshot(100));
  await page.locator('.sidebar .task-select').filter({ hasText: 'Compact metadata chat' }).click();
  const regular = await assertHeaders(page, 'desktop 100%', '.markdown');
  mkdirSync('verification', { recursive: true });
  await page.screenshot({ path: 'verification/compact-message-header-desktop.png' });
  await page.evaluate(value => window.__MONITTER_QA__.setSnapshot(value), snapshot(150));
  await expect(page.locator('.message-author').first()).toHaveCSS('font-size', '16.5px');
  const scaled = await assertHeaders(page, 'desktop 150%', '.markdown');
  expect(scaled.authorSize / regular.authorSize).toBeCloseTo(1.5, 1);
  await page.evaluate(value => window.__MONITTER_QA__.setSnapshot(value), { ...snapshot(100), settings: { ...snapshot(100).settings, theme: 'dark' } });
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await assertHeaders(page, 'desktop dark', '.markdown');
  await page.close();

  const lan = await browser.newPage({ viewport: { width: 760, height: 900 } });
  await lan.addInitScript(readFileSync('scripts/ui-fixture.js', 'utf8'));
  await lan.route('**/api/access', route => route.fulfill({ json: { required: false } }));
  await lan.route(url, async route => {
    const response = await route.fetch();
    await route.fulfill({ response, body: (await response.text()).replace('<html', '<html data-monitter-lan="1"') });
  });
  await lan.goto(url, { timeout: 90000 });
  await expect(lan.locator('.app-shell').first()).toBeVisible();
  await lan.evaluate(value => window.__MONITTER_QA__.setSnapshot(value), snapshot(100));
  await lan.locator('.sidebar .task-select').filter({ hasText: 'Compact metadata chat' }).click();
  const lanMetrics = await assertHeaders(lan, 'LAN narrow', '.markdown');
  await lan.screenshot({ path: 'verification/compact-message-header-lan.png' });
  expect(lanMetrics.authorSize).toBeCloseTo(regular.authorSize, 1);
  expect(lanMetrics.timeSize).toBeCloseTo(regular.timeSize, 1);
  await lan.close();
  return regular;
}

async function mobileCheck(baseUrl, relayUrl, browser) {
  const host = await browser.newPage();
  await host.addInitScript({ path: 'scripts/ui-fixture.js' });
  await host.goto(baseUrl, { timeout: 90000 });
  const invitation = await host.evaluate(async ({ relay, state }) => {
    const { createDesktopSession } = await import('/src/lib/controller/remote-client.ts');
    state.settings.interfaceFontSize = 14;
    state.settings.interfaceScale = 100;
    state.messages = [{ id: 'mobile-message', taskId: state.tasks[0].id, role: 'user', text: 'Mobile metadata body.', createdAt: Date.now() - 500 }];
    window.desktop = await createDesktopSession(relay, { getSnapshot: async () => state, sendMessage: async () => state, cancelTask: async () => state, resumeTask: async () => state, listTerminals: async () => [], readTerminal: async () => ({ chunks: [] }) });
    return window.desktop.invitation;
  }, { relay: relayUrl, state: snapshot(100) });
  const phone = await browser.newPage({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
  await phone.addInitScript({ path: 'scripts/ui-fixture.js' });
  await phone.goto(new URL('mobile', baseUrl).href, { timeout: 90000 });
  await expect(phone.getByRole('button', { name: 'Scan desktop code', exact: true })).toBeVisible();
  await phone.evaluate(value => window.dispatchEvent(new CustomEvent('monitter:qr', { detail: value })), invitation);
  await expect.poll(() => host.evaluate(() => window.desktop.getStatus())).toBe('pending');
  await host.evaluate(() => window.desktop.approve());
  await expect(phone.getByRole('button', { name: 'Compact metadata chat', exact: true })).toBeVisible();
  await phone.getByRole('button', { name: 'Compact metadata chat', exact: true }).click();
  const metrics = await assertHeaders(phone, 'mobile 100%', '.message-body');
  await phone.screenshot({ path: 'verification/compact-message-header-mobile.png' });
  await phone.close(); await host.close();
  return metrics;
}

let browser;
try {
  const relayUrl = await start(process.execPath, ['scripts/relay-server.mjs'], { MONITTER_RELAY_PORT: '0', MONITTER_RELAY_HOST: '127.0.0.1' }, /listening on (ws:\/\/[^\s]+)/);
  const baseUrl = await start(process.execPath, ['node_modules/vite/bin/vite.js', '--host', '127.0.0.1', '--port', '0'], { NO_COLOR: '1' }, /Local:\s+(http:\/\/[^\s]+)/);
  browser = await webkit.launch();
  const desktopMetrics = await desktopCheck(baseUrl, browser);
  const mobileMetrics = await mobileCheck(baseUrl, relayUrl, browser);
  expect(mobileMetrics.authorSize).toBeCloseTo(desktopMetrics.authorSize, 1);
  expect(mobileMetrics.timeSize).toBeCloseTo(desktopMetrics.timeSize, 1);
  console.log('Compact shared message metadata passed desktop, mobile, valid time, spacing, and 150% scale checks.');
} finally {
  await browser?.close();
  for (const child of children) {
    child.kill('SIGTERM');
    setTimeout(() => { if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL'); }, 2000).unref();
  }
}
