import { chromium, expect } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';

// Uses only the isolated browser fixture; no harness/account calls.
const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18420';
mkdirSync('verification', { recursive: true });
const browser = await chromium.launch({ headless: true });
const passed = [], errors = [];
let page;
try {
  page = await browser.newPage({ viewport: { width: 1280, height: 820 } });
  page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  const ready = async () => {
    await page.goto(url);
    await expect(page.getByRole('button', { name: 'New task', exact: true }).first()).toBeVisible({ timeout: 30000 });
  };
  const scale = () => page.evaluate(() => window.__MONITTER_QA__.snapshot().settings.interfaceScale);
  const callCount = method => page.evaluate(method => window.__MONITTER_QA__.calls.filter(call => call.method === method).length, method);
  const newDraft = async title => {
    await page.getByRole('button', { name: 'New task', exact: true }).first().click();
    if (title) {
      await page.locator('.draft-advanced').evaluate(el => el.open = true);
      await page.getByLabel('Task title', { exact: true }).fill(title);
    }
  };
  await ready();
  await page.keyboard.press('Meta+=');
  await expect.poll(scale).toBe(130);
  await page.keyboard.press('Meta+-');
  await expect.poll(scale).toBe(125);
  const prevented = await page.evaluate(() => {
    const event = new KeyboardEvent('keydown', { key: '+', code: 'Equal', metaKey: true, shiftKey: true, bubbles: true, cancelable: true });
    window.dispatchEvent(event); return event.defaultPrevented;
  });
  expect(prevented).toBe(true);
  await expect.poll(scale).toBe(130);
  passed.push('Cmd+=, Cmd+plus and Cmd+minus use saved 5% scale steps and prevent browser zoom');

  await page.evaluate(() => {
    const bridge = window.__MONITTER_BRIDGE__, original = bridge.saveSettings;
    let release;
    const pending = new Promise(resolve => release = resolve);
    window.__MONITTER_QA__.releaseScale = release;
    bridge.saveSettings = async (...args) => { bridge.saveSettings = original; await pending; return original(...args); };
    for (let i = 0; i < 3; i++) window.dispatchEvent(new KeyboardEvent('keydown', { key: '=', metaKey: true, bubbles: true, cancelable: true }));
  });
  expect(await scale()).toBe(130);
  await page.evaluate(() => window.__MONITTER_QA__.releaseScale());
  await expect.poll(scale).toBe(145);
  passed.push('rapid shortcuts preserve every increment while a settings save is pending');

  for (const [key, expected] of [['=', 200], ['-', 80]]) {
    await page.evaluate(key => {
      for (let i = 0; i < 40; i++) window.dispatchEvent(new KeyboardEvent('keydown', { key, metaKey: true, bubbles: true, cancelable: true }));
    }, key);
    await expect.poll(scale).toBe(expected);
  }
  await page.getByRole('button', { name: 'Monitter menu', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Preferences', exact: true }).click();
  await expect(page.getByRole('slider', { name: 'Interface scale' })).toHaveValue('80');
  await page.keyboard.press('Meta+=');
  await expect(page.getByRole('slider', { name: 'Interface scale' })).toHaveValue('85');
  await page.getByRole('button', { name: 'Close', exact: true }).click();
  passed.push('scale shortcuts clamp to 80–200% and stay synchronized with the settings slider');

  await ready();
  await newDraft();
  const composer = page.getByLabel('Task message', { exact: true });
  await page.screenshot({ path: 'verification/ui-new-draft.png' });
  await composer.fill('/se');
  await expect(page.getByRole('menu', { name: 'Monitter commands' })).toBeVisible();
  await expect(page.getByRole('menuitem')).toHaveCount(1);
  await expect(page.getByRole('menuitem')).toContainText('/settings');
  await composer.press('Enter');
  await expect(page.getByRole('dialog')).toBeVisible();
  await page.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(composer).toHaveValue('');
  expect(await callCount('createTask')).toBe(0);
  expect(await callCount('sendMessage')).toBe(0);
  passed.push('slash menu filters and executes app settings without creating or messaging an agent');

  await composer.fill('/');
  await composer.press('ArrowDown');
  await composer.press('ArrowUp');
  await composer.press('ArrowDown');
  await composer.press('Enter');
  await expect(page.getByRole('dialog')).toBeVisible();
  await page.getByRole('button', { name: 'Close', exact: true }).click();
  await composer.fill('/project');
  await composer.press('Enter');
  await expect(page.getByLabel('Project', { exact: true })).toBeFocused();
  passed.push('slash arrow navigation and project selection work without sending command text');

  await composer.fill('/compact');
  await composer.press('Escape');
  await expect(page.getByRole('menu')).toHaveCount(0);
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(page.getByText(/This command is not available through Monitter/)).toBeVisible();
  await expect(composer).toHaveValue('/compact');
  await composer.press('Meta+Enter');
  expect(await callCount('createTask')).toBe(0);
  expect(await callCount('sendMessage')).toBe(0);
  await composer.fill('/goal build everything');
  await composer.press('Enter');
  expect(await callCount('getTaskGoal')).toBe(0);
  expect(await callCount('sendMessage')).toBe(0);
  passed.push('unsupported native commands remain unsent from both Send and keyboard paths');

  await composer.fill('/settings');
  await composer.dispatchEvent('keydown', { key: 'Enter', isComposing: true, bubbles: true });
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await composer.fill('//literal command explanation');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect.poll(() => callCount('sendMessage')).toBe(1);
  const sent = await page.evaluate(() => window.__MONITTER_QA__.calls.find(call => call.method === 'sendMessage'));
  expect(sent.args.text).toBe('/literal command explanation');
  expect(await page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks[0].title)).toBe('/literal command explanation');
  passed.push('IME is respected, explicit slash escaping sends literal text, and first message names the chat');

  await ready();
  await newDraft('Pending draft A');
  await composer.fill('First message from A');
  await newDraft('Unrelated draft B');
  await composer.fill('Keep B text');
  await page.getByRole('button', { name: 'Pending draft A', exact: true }).click();
  await page.evaluate(() => {
    const bridge = window.__MONITTER_BRIDGE__, original = bridge.createTask;
    let release;
    const pending = new Promise(resolve => release = resolve);
    window.__MONITTER_QA__.releaseCreate = release;
    bridge.createTask = async (...args) => { bridge.createTask = original; await pending; return original(...args); };
  });
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await composer.fill('New text written while A was sending');
  await page.getByRole('button', { name: 'Unrelated draft B', exact: true }).click();
  await page.evaluate(() => window.__MONITTER_QA__.releaseCreate());
  await expect.poll(() => callCount('sendMessage')).toBe(1);
  await expect(composer).toHaveValue('Keep B text');
  await page.getByRole('button', { name: 'Pending draft A', exact: true }).first().click();
  await expect(composer).toHaveValue('New text written while A was sending');
  expect(await callCount('createTask')).toBe(1);
  passed.push('new typing during first-send survives switching drafts and the completed draft converts to its task');

  await ready();
  await page.getByRole('button', { name: 'Monitter menu', exact: true }).click();
  await expect(page.getByRole('menuitem', { name: 'Preferences', exact: true })).toBeVisible();
  await expect(page.getByRole('menuitem', { name: 'Hosts', exact: true })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('menuitem', { name: 'Hosts', exact: true })).toHaveCount(0);
  await expect(page.locator('.host-switch')).toHaveCount(0);
  passed.push('brand dropdown contains Preferences/Hosts and replaces the separate host row');

  await page.evaluate(() => {
    const qa = window.__MONITTER_QA__, s = qa.snapshot(), now = Date.now();
    s.tasks.push({id:'identity-chat',agentId:'atlas',title:'A compact subject',nativeSessionId:'qa-session',status:'idle',archived:false,createdAt:now,updatedAt:now,parentTaskId:null,channelId:null,projectId:null,hostId:'local',cwd:'/tmp/identity-folder',provider:'codex',model:'qa-model',sandbox:'read-only'});
    qa.setSnapshot(s);
  });
  await page.getByRole('button', { name: 'A compact subject', exact: true }).first().click();
  const heading = page.locator('.task-heading');
  await expect(heading.getByRole('heading', { name: 'A compact subject', exact: true })).toBeVisible();
  expect((await heading.boundingBox()).height).toBeLessThanOrEqual(70);
  await expect(heading).not.toContainText('/tmp/identity-folder');
  await expect(page.locator('.run-detail')).toContainText('/tmp/identity-folder');
  await expect(page.locator('.run-detail')).toContainText('qa-model');
  await page.getByRole('button', { name: 'Task actions', exact: true }).click();
  await page.getByRole('button', { name: 'Hide run detail', exact: true }).click();
  await expect(page.locator('.run-detail')).toHaveCount(0);
  await page.getByRole('button', { name: 'Task actions', exact: true }).click();
  await page.getByRole('button', { name: 'Show run detail', exact: true }).click();
  await expect(page.locator('.run-detail')).toBeVisible();
  passed.push('compact subject header moves folder/model into identity and retains run-detail controls');

  await page.getByRole('button', { name: 'Change avatar', exact: true }).click();
  let dialog = page.getByRole('dialog');
  const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jp1sAAAAASUVORK5CYII=', 'base64');
  await dialog.getByLabel('Avatar image', { exact: true }).setInputFiles({name:'avatar.png',mimeType:'image/png',buffer:png});
  await expect(dialog.getByAltText('Current avatar')).toBeVisible();
  await dialog.getByRole('button', { name: 'Save agent', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().agents[0].avatar)).toMatch(/^data:image\/png;base64,/);
  await expect(page.locator('.agent-identity img')).toBeVisible();
  await page.screenshot({ path: 'verification/ui-compact-header-identity.png' });
  await page.getByRole('button', { name: 'Change avatar', exact: true }).click();
  dialog = page.getByRole('dialog');
  await dialog.getByLabel('Avatar image', { exact: true }).setInputFiles({name:'not-an-avatar.svg',mimeType:'image/svg+xml',buffer:Buffer.from('<svg/>')});
  await expect(page.getByText('Choose a PNG, JPEG, or WebP image up to 2 MiB.', { exact: true })).toBeVisible();
  await dialog.getByRole('button', { name: 'Remove avatar', exact: true }).click();
  await dialog.getByRole('button', { name: 'Save agent', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().agents[0].avatar)).toBe(null);
  passed.push('agent avatar uploads and removes locally, rejecting unsupported file types');

  await ready();
  await newDraft();
  await page.setViewportSize({ width: 512, height: 340 });
  await expect(composer).toBeVisible();
  const layout = await page.evaluate(() => ({
    documentHeight: document.documentElement.scrollHeight,
    viewportHeight: window.innerHeight,
    documentWidth: document.documentElement.scrollWidth,
    viewportWidth: window.innerWidth,
    paneOverflow: getComputedStyle(document.querySelector('.draft-layout')).overflowY,
  }));
  expect(layout.documentHeight).toBeLessThanOrEqual(layout.viewportHeight);
  expect(layout.documentWidth).toBeLessThanOrEqual(layout.viewportWidth);
  expect(layout.paneOverflow).toBe('auto');
  await composer.fill('Draft at compact scale');
  await page.getByRole('button', { name: 'Send', exact: true }).scrollIntoViewIfNeeded();
  await page.screenshot({ path: 'verification/ui-new-draft-compact.png' });
  passed.push('new-chat view keeps document fixed and independently scrolls at a compact 200%-equivalent viewport');

  expect(errors).toEqual([]);
  const report = { completedAt: new Date().toISOString(), passed, errors };
  writeFileSync('verification/ui-controls-results.json', JSON.stringify(report, null, 2) + '\n');
  console.log(JSON.stringify(report, null, 2));
} catch (error) {
  writeFileSync('verification/ui-controls-results.json', JSON.stringify({ passed, errors, error: String(error) }, null, 2) + '\n');
  if (page) await page.screenshot({ path: 'verification/ui-controls-failure.png' });
  throw error;
} finally { await browser.close(); }
