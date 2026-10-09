// Synthetic, read-only visual baseline for the current Tauri desktop UI.
// Requires the parent-owned localhost fixture Vite server (default port 19463).
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { chromium, expect } from '@playwright/test';

const argIndex = process.argv.indexOf('--url');
const url = argIndex >= 0 ? process.argv[argIndex + 1] : 'http://127.0.0.1:19463/';
if (!url) throw new Error('--url requires a URL');
const parsedUrl = new URL(url);
if (!['127.0.0.1', 'localhost', '[::1]'].includes(parsedUrl.hostname)) {
  throw new Error('Visual reference accepts localhost only; remote hosts are blocked.');
}
const outputDir = path.resolve('artifacts/gpui/reference');
await mkdir(outputDir, { recursive: true });
const sourceCommit = execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
const browser = await chromium.launch({ headless: true });
const results = [];

const snapshot = () => {
  const now = 1_790_000_000_000;
  const taskId = 'visual-reference-chat';
  return {
    hosts: [{ id: 'local', name: 'This Mac', kind: 'local', address: '', user: '', port: 0, identityFile: '', defaultCwd: '/tmp/monitter-visual-reference', codexPath: 'codex', claudePath: '', opencodePath: '', hermesPath: '' }],
    agents: [{ id: 'visual-reference-agent', avatar: null, name: 'Reference Agent', description: 'Synthetic visual fixture', instructions: '', provider: 'codex', model: 'fixture-model', hostId: 'local', cwd: '/tmp/monitter-visual-reference', color: '#00A8F0', sandbox: 'read-only', expertise: [], responsibilities: [], skills: [], collaborationEnabled: false }],
    tasks: [{ id: taskId, agentId: 'visual-reference-agent', title: 'Visual reference conversation', nativeSessionId: null, status: 'idle', archived: false, createdAt: now, updatedAt: now, parentTaskId: null, channelId: null, projectId: null, hostId: 'local', cwd: '/tmp/monitter-visual-reference', provider: 'codex', model: 'fixture-model', sandbox: 'read-only', codexHome: null }],
    messages: [
      { id: 'visual-reference-user', taskId, role: 'user', text: 'Please summarize the three visual checks for this synthetic reference.', createdAt: now - 20_000, attachments: [] },
      { id: 'visual-reference-assistant', taskId, role: 'assistant', text: 'This is synthetic fixture content for layout measurement. The response uses a short paragraph and a second line so the transcript spacing and message width remain visible in both reference sizes.', createdAt: now - 10_000, attachments: [] },
    ],
    events: [], channels: [], projects: [], collaborations: [], queuedMessages: [], approvalRequests: [], approvalRules: [],
    settings: {
      accent: '#00A8F0', theme: 'light', interfaceScale: 125, interfaceDensity: 'normal',
      interfaceFont: 'IBM Plex Sans', interfaceFontSize: 14, interfaceFontWeight: 400,
      chatFont: 'IBM Plex Sans', chatFontSize: 13, chatFontWeight: 400, chatLineHeight: 1.65,
      terminalFont: 'IBM Plex Mono', terminalFontSize: 14, terminalFontWeight: 400, terminalLineHeight: 1,
      windowSurface: 'opaque', windowTransparency: 18, showToolActivity: true,
      showReasoningSummaries: true, sendWithEnter: false, sidebarView: 'standard',
    },
  };
};

try {
  for (const mode of ['light', 'dark']) {
    for (const size of [{ width: 1200, height: 820 }, { width: 1600, height: 900 }]) {
      const page = await browser.newPage({ viewport: size, deviceScaleFactor: 1 });
      const errors = [];
      page.on('pageerror', error => errors.push(error.message));
      await page.route('**/*', route => {
        const requestUrl = new URL(route.request().url());
        if (['127.0.0.1', 'localhost', '[::1]'].includes(requestUrl.hostname) || ['data:', 'blob:'].includes(requestUrl.protocol)) return route.continue();
        return route.abort('blockedbyclient');
      });
      await page.addInitScript(() => {
        // Renderer compatibility shim only; all bridge replies come from ui-fixture.js.
        window.__TAURI_INTERNALS__ = { transformCallback: () => 1, invoke: async () => 1 };
        localStorage.setItem('monitter.appearance.app-theme-pair.v2', JSON.stringify({ light: 'monitter', dark: 'monitter', accent: null, contrast: 0 }));
        localStorage.setItem('monitter.appearance.surface-tint.v1', '5');
        localStorage.setItem('monitter.appearance.border-opacity.v1', '100');
        localStorage.setItem('monitter.interface-scale.v1:desktop', '125');
      });
      await page.addInitScript({ path: 'scripts/ui-fixture.js' });
      await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 90_000 });
      await expect(page.locator('.app-shell')).toBeVisible({ timeout: 60_000 });
      await page.evaluate(({ mode, data }) => {
        data.settings.theme = mode;
        window.__MONITTER_QA__.setSnapshot(data);
      }, { mode, data: snapshot() });
      await page.getByRole('button', { name: 'Open chat Visual reference conversation', exact: true }).click();
      await expect(page.locator('.message').filter({ hasText: 'synthetic fixture content' })).toBeVisible({ timeout: 30_000 });
      await page.evaluate(() => document.fonts.ready);
      await page.waitForTimeout(250);
      const measurements = await page.evaluate(() => {
        const rect = selector => {
          const node = document.querySelector(selector);
          if (!node) return null;
          const box = node.getBoundingClientRect();
          const style = getComputedStyle(node);
          return {
            x: Math.round(box.x * 100) / 100, y: Math.round(box.y * 100) / 100,
            width: Math.round(box.width * 100) / 100, height: Math.round(box.height * 100) / 100,
            background: style.backgroundColor, color: style.color, borderColor: style.borderColor,
            borderRadius: style.borderRadius, fontFamily: style.fontFamily, fontSize: style.fontSize,
          };
        };
        return {
          viewport: { width: innerWidth, height: innerHeight, devicePixelRatio },
          shell: rect('.app-shell'), sidebar: rect('.sidebar'), topbar: rect('.topbar'),
          conversationHeader: rect('.conversation-head'), composer: rect('.composer-box'),
          messages: [...document.querySelectorAll('.message')].map(node => {
            const box = node.getBoundingClientRect(), style = getComputedStyle(node);
            return { role: node.classList.contains('user') ? 'user' : node.classList.contains('final-answer') ? 'final-answer' : 'assistant', x: Math.round(box.x * 100) / 100, y: Math.round(box.y * 100) / 100, width: Math.round(box.width * 100) / 100, height: Math.round(box.height * 100) / 100, background: style.backgroundColor, borderColor: style.borderColor, borderRadius: style.borderRadius, padding: style.padding };
          }),
          theme: document.documentElement.dataset.theme,
          appThemeLight: document.documentElement.dataset.appThemeLight,
          appThemeDark: document.documentElement.dataset.appThemeDark,
          surfaceTint: getComputedStyle(document.documentElement).getPropertyValue('--surface-tint').trim(),
          borderOpacity: getComputedStyle(document.documentElement).getPropertyValue('--border-opacity').trim(),
          interfaceScale: getComputedStyle(document.documentElement).getPropertyValue('--interface-scale').trim(),
          interfaceFont: getComputedStyle(document.documentElement).getPropertyValue('--interface-font').trim(),
          chatFont: getComputedStyle(document.documentElement).getPropertyValue('--chat-font').trim(),
        };
      });
      assert.equal(measurements.theme, mode);
      assert.equal(measurements.appThemeLight, 'monitter');
      assert.equal(measurements.appThemeDark, 'monitter');
      const label = `tauri-${mode}-${size.width}x${size.height}`;
      await page.screenshot({ path: path.join(outputDir, `${label}.png`), animations: 'disabled' });
      results.push({ label, measurements, pageErrors: errors });
      await page.close();
    }
  }
} finally {
  await browser.close();
}

const metadata = {
  label: 'SYNTHETIC UI FIXTURE — not live Monitter data or agent output',
  sourceCommit,
  url: parsedUrl.origin + parsedUrl.pathname,
  viewportPixels: [{ width: 1200, height: 820 }, { width: 1600, height: 900 }],
  modes: ['light', 'dark'],
  profile: { appThemeLight: 'monitter', appThemeDark: 'monitter', customAccent: null, contrast: 0, surfaceTint: 5, borderOpacity: 100, interfaceScale: 125, density: 'normal', interfaceFont: 'IBM Plex Sans', interfaceFontSize: 14, chatFont: 'IBM Plex Sans', chatFontSize: 13, chatLineHeight: 1.65 },
  fixture: { agent: 'Reference Agent', task: 'Visual reference conversation', messages: ['synthetic user prompt', 'synthetic assistant response'], bridge: 'scripts/ui-fixture.js in-memory only' },
  captures: results,
};
await writeFile(path.join(outputDir, 'metadata.json'), JSON.stringify(metadata, null, 2) + '\n');
assert.deepEqual(results.flatMap(item => item.pageErrors), [], 'No renderer errors in reference pages');
console.log(`Wrote four synthetic Tauri reference captures and metadata to ${outputDir}`);
