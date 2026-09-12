import { webkit, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';

const testUrl = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18449/';
const browser = await webkit.launch({ headless: true });

const tasks = [
  {
    id: '11111111-1111-4111-8111-111111111111',
    agentId: 'atlas', title: 'OpenChamber Permission Troubleshooting — long toolbar title', status: 'idle', archived: false,
    updatedAt: 2, createdAt: 1, nativeSessionId: null, parentTaskId: null,
    channelId: null, projectId: null, hostId: 'local', cwd: '/tmp/monitter-ui-test',
    provider: 'codex', model: '', modelSettings: null, sandbox: 'read-only',
  },
  {
    id: '22222222-2222-4222-8222-222222222222',
    agentId: 'atlas', title: 'Second toolbar chat', status: 'idle', archived: false,
    updatedAt: 1, createdAt: 1, nativeSessionId: null, parentTaskId: null,
    channelId: null, projectId: null, hostId: 'local', cwd: '/tmp/monitter-ui-test',
    provider: 'codex', model: '', modelSettings: null, sandbox: 'read-only',
  },
];

function snapshot() {
  return {
    hosts: [{ id: 'local', name: 'This Mac', kind: 'local', address: '', user: '', port: 0,
      identityFile: '', defaultCwd: '/tmp/monitter-ui-test', codexPath: 'codex',
      claudePath: '', opencodePath: '', hermesPath: '' }],
    agents: [{ id: 'atlas', avatar: null, name: 'Atlas', description: 'Coding partner',
      instructions: 'Work carefully.', provider: 'codex', model: '', hostId: 'local',
      cwd: '/tmp/monitter-ui-test', color: '#397e61', sandbox: 'read-only',
      expertise: [], responsibilities: [], skills: [], collaborationEnabled: true }],
    tasks,
    messages: [], events: [], channels: [], projects: [], collaborations: [], queuedMessages: [],
    settings: { accent: '#3f9d6a', theme: 'light', interfaceScale: 100, showToolActivity: true,
      showReasoningSummaries: true, sendWithEnter: false, sidebarView: 'standard' },
  };
}

async function newPage(width) {
  const page = await browser.newPage({ viewport: { width, height: 844 }, isMobile: true, hasTouch: true });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(readFileSync('scripts/ui-fixture.js', 'utf8'));
  await page.goto(testUrl, { timeout: 90000 });
  const shell = page.locator('.app-shell').first();
  await expect(shell).toHaveClass(/mobile-navigation/, { timeout: 60000 });
  await page.evaluate(value => window.__MONITTER_QA__.setSnapshot(value), snapshot());
  await expect(page.getByRole('button', { name: 'OpenChamber Permission Troubleshooting — long toolbar title', exact: true })).toBeVisible();
  return { page, shell, errors };
}

async function assertSameRow(page, selectors) {
  const boxes = await Promise.all(selectors.map(selector => page.locator(selector).first().boundingBox()));
  expect(boxes.every(Boolean)).toBe(true);
  const top = Math.max(...boxes.map(box => box.y));
  const bottom = Math.min(...boxes.map(box => box.y + box.height));
  expect(bottom).toBeGreaterThan(top);
  for (const box of boxes) {
    expect(box.x + box.width).toBeLessThanOrEqual((await page.evaluate(() => innerWidth)) + 1);
    expect(box.x).toBeGreaterThanOrEqual(-1);
  }
}

async function exerciseMobile(width) {
  const { page, shell, errors } = await newPage(width);
  try {
    await page.getByRole('button', { name: 'OpenChamber Permission Troubleshooting — long toolbar title', exact: true }).click();
    await expect(shell).toHaveClass(/mobile-main/);
    const toolbar = page.locator('.topbar').first();
    await expect(toolbar).toBeVisible();
    await expect(toolbar.getByRole('button', { name: 'Back to chats', exact: true })).toBeVisible();
    await expect(toolbar.locator('.tab-picker-trigger')).toBeVisible();
    await expect(toolbar.getByRole('button', { name: 'Open terminal', exact: true })).toBeVisible();
    await expect(toolbar.locator('.workspace-context .avatar')).toHaveCount(0);
    await assertSameRow(page, [
      '.topbar button[aria-label="Back to chats"]',
      '.topbar .tab-picker-trigger',
      '.topbar button[aria-label="Open terminal"]',
    ]);

    await page.getByRole('button', { name: 'Back to chats', exact: true }).click();
    await expect(page.getByRole('complementary', { name: 'Agents and tasks' })).toBeVisible();
    await page.getByRole('button', { name: 'Second toolbar chat', exact: true }).click();
    await page.locator('.tab-picker-trigger').click();
    await expect(page.locator('.tab-picker-list')).toBeVisible();
    await page.locator('.tab-picker-list [data-tab-id="11111111-1111-4111-8111-111111111111"] .tab').click();
    await expect(page.locator('.tab-picker-trigger')).toContainText('OpenChamber Permission Troubleshooting');
    if (width === 390) await page.screenshot({ path: 'verification/compact-mobile-toolbar.png' });

    await page.getByRole('button', { name: 'Open terminal', exact: true }).click();
    await expect(page.locator('.terminal-pane')).toBeVisible();
    await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.calls.some(call => call.method === 'openTerminal'))).toBe(true);
    expect(errors).toEqual([]);
    console.log(`WebKit ${width}px compact mobile toolbar checks passed.`);
  } finally {
    await page.close();
  }
}

try {
  await exerciseMobile(390);
  await exerciseMobile(320);
  await exerciseMobile(760);

  const desktop = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  const desktopErrors = [];
  desktop.on('pageerror', error => desktopErrors.push(error.message));
  await desktop.addInitScript(readFileSync('scripts/ui-fixture.js', 'utf8'));
  try {
    await desktop.goto(testUrl, { timeout: 90000 });
    await expect(desktop.locator('.app-shell').first()).not.toHaveClass(/mobile-navigation/, { timeout: 60000 });
    await desktop.evaluate(value => window.__MONITTER_QA__.setSnapshot(value), snapshot());
    await desktop.getByRole('button', { name: 'OpenChamber Permission Troubleshooting — long toolbar title', exact: true }).click();
    await expect(desktop.locator('.topbar').first()).toBeVisible();
    await expect(desktop.locator('.topbar').first().getByRole('button', { name: 'Back to chats', exact: true })).toHaveCount(0);
    await expect(desktop.locator('.topbar [data-workspace-context]')).toBeVisible();
    expect(desktopErrors).toEqual([]);
    console.log('WebKit desktop toolbar context checks passed.');
  } finally {
    await desktop.close();
  }
} finally {
  await browser.close();
}
