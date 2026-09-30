import { chromium, expect, webkit } from '@playwright/test';

// Browser fixture only. Seed a representative retained workspace, then verify
// typing is persisted without walking that retained payload on every key.
const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18420';
const browserType = process.env.MONITTER_TEST_BROWSER === 'webkit' ? webkit : chromium;
const browser = await browserType.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1280, height: 820 } });
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  const retainedDrafts = Object.fromEntries(Array.from({ length: 100 }, (_, index) => [`task:retained-${index}`, 'x'.repeat(10_000)]));
  const previewDataUrl = `data:image/png;base64,${'A'.repeat(256 * 1024)}`;
  const queuedAttachments = { retained: Array.from({ length: 4 }, (_, index) => ({
    id: `retained-image-${index}`, name: `retained-image-${index}.png`, mimeType: 'image/png', size: 1, path: `/tmp/image-${index}.png`, previewDataUrl,
  })) };
  await page.addInitScript(({ drafts, attachments }) => {
    const pane = { drafts, queuedAttachments: attachments };
    const workspace = { version: 1, layout: { id: 'main' }, activePaneId: 'main', main: pane, panes: {}, sidebarCollapsed: false, collapsedAgents: {}, collapsedProjects: {}, terminals: [] };
    if (!sessionStorage.getItem('workspace-typing-test-seeded')) {
      localStorage.setItem('monitter.workspaces.v2', JSON.stringify({ version: 2, activeWorkspaceKey: 'all', workspaces: { all: workspace } }));
      sessionStorage.setItem('workspace-typing-test-seeded', 'true');
    }
    const qa = window.__WORKSPACE_TYPING_QA__ = { retainedPayloadSerializations: 0, persistenceSnapshots: 0, lastBytes: 0 };
    const stringify = JSON.stringify;
    JSON.stringify = function(value, replacer, space) {
      const result = stringify.call(this, value, replacer, space);
      if (value && typeof value === 'object' && Object.hasOwn(value, 'drafts') && Object.hasOwn(value, 'queuedAttachments')) {
        // The broad reactive dependency walk has these fields at top level.
        // Debounced persistence also snapshots them, alongside settingsEditor.
        if (Object.hasOwn(value, 'agentDraft')) qa.retainedPayloadSerializations += 1;
        else qa.persistenceSnapshots += 1;
        qa.lastBytes = result.length;
      }
      return result;
    };
  }, { drafts: retainedDrafts, attachments: queuedAttachments });

  await page.goto(url);
  const newChat = page.locator('.pane-grid').getByRole('button', { name: 'New chat', exact: true }).first();
  await expect(newChat).toBeVisible({ timeout: 30000 });
  await page.waitForTimeout(350);
  await newChat.click();
  const composer = page.getByRole('textbox', { name: 'Task message', exact: true });
  await expect(composer).toBeVisible();
  // Let navigation and initial Svelte effects settle before measuring typing.
  // WebKit can flush one queued effect later than Chromium.
  await page.evaluate(async () => {
    let last = window.__WORKSPACE_TYPING_QA__.retainedPayloadSerializations;
    let stableSamples = 0;
    for (let attempt = 0; attempt < 40 && stableSamples < 5; attempt += 1) {
      await new Promise(resolve => setTimeout(resolve, 100));
      const current = window.__WORKSPACE_TYPING_QA__.retainedPayloadSerializations;
      stableSamples = current === last ? stableSamples + 1 : 0;
      last = current;
    }
    if (stableSamples < 5) throw new Error('retained-state serialization did not settle before typing');
  });

  const baseline = await page.evaluate(() => window.__WORKSPACE_TYPING_QA__.retainedPayloadSerializations);
  const payload = await page.evaluate(() => {
    const main = JSON.parse(localStorage.getItem('monitter.workspaces.v2')).workspaces.all.main;
    return { bytes: JSON.stringify({ drafts: main.drafts, queuedAttachments: main.queuedAttachments }).length,
      retainedDraftCount: Object.keys(main.drafts).filter(key => key.startsWith('task:retained-')).length,
      attachmentCount: Object.values(main.queuedAttachments).flat().length };
  });
  expect(payload.bytes).toBeGreaterThan(1_000_000);
  expect(payload.retainedDraftCount).toBe(100);
  expect(payload.attachmentCount).toBe(4);
  await composer.pressSequentially('Keep this draft after reload.');
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  expect(await page.evaluate(() => window.__WORKSPACE_TYPING_QA__.retainedPayloadSerializations)).toBe(baseline);

  await expect.poll(() => page.evaluate(() => {
    const saved = JSON.parse(localStorage.getItem('monitter.workspaces.v2') || 'null');
    return Object.values(saved?.workspaces?.all?.main?.drafts ?? {}).includes('Keep this draft after reload.');
  }), { timeout: 3000 }).toBe(true);
  const snapshots = await page.evaluate(() => window.__WORKSPACE_TYPING_QA__.persistenceSnapshots);
  expect(snapshots).toBeGreaterThan(0);
  await page.reload();
  await expect(page.getByRole('textbox', { name: 'Task message', exact: true })).toHaveValue('Keep this draft after reload.', { timeout: 30000 });
  console.log(`Workspace typing: ${payload.bytes.toLocaleString()} retained characters across ${payload.retainedDraftCount} drafts and ${payload.attachmentCount} previews; the broad dependency walk stayed at ${baseline} calls while debounced persistence captured ${snapshots} time(s), and the composer survived reload.`);
} finally {
  await browser.close();
}
