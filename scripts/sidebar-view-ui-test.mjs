import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { webkit, expect as baseExpect } from '@playwright/test';

const expect = baseExpect.configure({ timeout: 120000 });
const children = [];
function startVite() {
  const child = spawn(process.execPath, ['node_modules/vite/bin/vite.js', '--host', '127.0.0.1', '--port', '0'], {
    cwd: process.cwd(), env: { ...process.env, NO_COLOR: '1' }, stdio: ['ignore', 'pipe', 'pipe'],
  });
  children.push(child);
  return new Promise((resolve, reject) => {
    let output = '';
    const timer = setTimeout(() => reject(Error(`Vite startup timed out: ${output.slice(-1000)}`)), 60000);
    const receive = data => {
      output += data.toString();
      const match = output.match(/Local:\s+(http:\/\/[^\s]+)/);
      if (match) { clearTimeout(timer); resolve(match[1]); }
    };
    child.stdout.on('data', receive); child.stderr.on('data', receive);
    child.once('error', reason => { clearTimeout(timer); reject(reason); });
    child.once('exit', code => { clearTimeout(timer); reject(Error(`Vite exited ${code}: ${output.slice(-1000)}`)); });
  });
}

function installBridge(context, initialView) {
  return context.addInitScript(view => {
    localStorage.setItem('monitter.sidebar-view.v2:web', view);
    window.__settingsWrites = 0;
    const snapshot = {
      hosts: [], agents: [], tasks: [], messages: [], events: [], channels: [], projects: [],
      collaborations: [], queuedMessages: [], approvalRequests: [],
      settings: { accent: '#3f9d6a', theme: 'dark', interfaceScale: 125, interfaceDensity: 'normal', showToolActivity: true, showReasoningSummaries: true, sendWithEnter: false, sidebarView: 'standard', busyMessageMode: 'queue' },
    };
    window.__MONITTER_TEST_BRIDGE__ = {
      invoke: async command => {
        if (command === 'get_snapshot') return structuredClone(snapshot);
        if (command === 'save_settings') { window.__settingsWrites++; return structuredClone(snapshot); }
        if (command === 'list_terminals') return [];
        if (command === 'get_task_events') return { events: [], nextBefore: null };
        return null;
      },
      listen: async () => () => {},
    };
  }, initialView);
}

let browser;
try {
  const baseUrl = await startVite();
  browser = await webkit.launch({ headless: true });
  const projectsClient = await browser.newContext({ viewport: { width: 1200, height: 800 } });
  const standardClient = await browser.newContext({ viewport: { width: 1200, height: 800 } });
  await installBridge(projectsClient, 'projects');
  await installBridge(standardClient, 'standard');
  const projectsPage = await projectsClient.newPage();
  const standardPage = await standardClient.newPage();
  const pageErrors = [];
  const consoleErrors = [];
  projectsPage.on('pageerror', error => pageErrors.push(error.message));
  standardPage.on('pageerror', error => pageErrors.push(error.message));
  projectsPage.on('console', message => { if (message.type() === 'error') consoleErrors.push(message.text()); });
  standardPage.on('console', message => { if (message.type() === 'error') consoleErrors.push(message.text()); });
  const responses = await Promise.all([projectsPage.goto(baseUrl), standardPage.goto(baseUrl)]);

  await expect(projectsPage.locator('.view-toggle')).toHaveCount(3, { timeout: 240000 }).catch(async error => {
    throw new Error(`${error.message}\nPage errors: ${pageErrors.join(' | ')}\nConsole: ${consoleErrors.join(' | ')}\nBody: ${(await projectsPage.locator('body').innerText()).slice(0, 2000)}`);
  });
  if (responses[0]?.status() !== 200) throw new Error(`Workspace returned ${responses[0]?.status()}`);
  assert.deepEqual(pageErrors, []);

  await expect(projectsPage.getByRole('button', { name: 'Projects view' })).toHaveAttribute('aria-pressed', 'true');
  await expect(standardPage.getByRole('button', { name: 'Standard view' })).toHaveAttribute('aria-pressed', 'true');

  const started = Date.now();
  await projectsPage.getByRole('button', { name: 'Activity view' }).click();
  await expect(projectsPage.getByRole('button', { name: 'Activity view' })).toHaveAttribute('aria-pressed', 'true');
  assert.ok(Date.now() - started < 1000, 'Sidebar view should change without a backend round trip');
  assert.equal(await projectsPage.evaluate(() => localStorage.getItem('monitter.sidebar-view.v2:web')), 'activity');
  assert.equal(await projectsPage.evaluate(() => window.__settingsWrites), 0);
  await expect(standardPage.getByRole('button', { name: 'Standard view' })).toHaveAttribute('aria-pressed', 'true');
  assert.equal(await standardPage.evaluate(() => localStorage.getItem('monitter.sidebar-view.v2:web')), 'standard');

  const siblingPage = await projectsClient.newPage();
  await siblingPage.goto(baseUrl);
  await expect(siblingPage.getByRole('button', { name: 'Projects view' })).toHaveAttribute('aria-pressed', 'true');
  await expect(projectsPage.getByRole('button', { name: 'Activity view' })).toHaveAttribute('aria-pressed', 'true');
  await projectsPage.reload();
  await expect(projectsPage.getByRole('button', { name: 'Activity view' })).toHaveAttribute('aria-pressed', 'true');
  await expect(siblingPage.getByRole('button', { name: 'Projects view' })).toHaveAttribute('aria-pressed', 'true');
  assert.equal(await projectsPage.evaluate(() => window.__settingsWrites), 0);

  await Promise.all([projectsClient.close(), standardClient.close()]);
  console.log('Sidebar view UI: independent clients and same-origin tabs preserve their choices across reloads; switching performs no settings write.');
} finally {
  await browser?.close();
  for (const child of children) child.kill('SIGTERM');
}
