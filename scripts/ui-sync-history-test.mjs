import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { fileURLToPath } from 'node:url';
import { build } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { webkit, expect } from '@playwright/test';

const root = process.cwd();
const iconStub = fileURLToPath(new URL('./fixtures/lucide-stub.svelte', import.meta.url));
const result = await build({ configFile: false, plugins: [svelte()], root,
  resolve: { alias: [
    { find: '$lib', replacement: `${root}/src/lib` },
    { find: /^@lucide\/svelte(?:\/.*)?$/, replacement: iconStub },
  ] },
  build: { write: false, minify: false, rollupOptions: { input: 'scripts/fixtures/ui-sync-history-entry.js' } },
});
const files = new Map(result.output.map(file => [`/${file.fileName}`, file.type === 'asset' ? file.source : file.code]));
const entry = result.output.find(file => file.type === 'chunk' && file.isEntry);
if (!entry) throw new Error('No UI sync history fixture entry was built');
const styles = result.output.filter(file => file.type === 'asset' && file.fileName.endsWith('.css'));
files.set('/', `<!doctype html><meta name="viewport" content="width=device-width, initial-scale=1"><div id="app"></div>${styles.map(file => `<link rel="stylesheet" href="/${file.fileName}">`).join('')}<script type="module" src="/${entry.fileName}"></script>`);
const server = createServer((request, response) => {
  const body = files.get(request.url?.split('?')[0] || '/');
  if (body === undefined) return response.writeHead(404).end();
  response.writeHead(200, { 'content-type': request.url === '/' ? 'text/html' : request.url?.endsWith('.css') ? 'text/css' : 'text/javascript' });
  response.end(body);
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const address = server.address();
if (!address || typeof address === 'string') throw new Error('No UI sync fixture server address');
const browser = await webkit.launch({ headless: true });
try {
  const context = await browser.newContext({ viewport: { width: 900, height: 720 } });
  await context.addInitScript(() => {
    const taskId = '11111111-1111-4111-8111-111111111111';
    const messages = Array.from({ length: 100 }, (_, index) => ({ id: `m${String(index + 1).padStart(3, '0')}`, taskId, role: index % 2 ? 'assistant' : 'user', text: `message-${index + 1}`, createdAt: index + 1, attachments: [] }));
    let revision = 'rev-1'; let delta = null; let staleGap = false; let delayPage = false; let releasePage; let pendingPageResolve;
    const pageGate = new Promise(resolve => { releasePage = resolve; });
    const emptySnapshot = () => ({ hosts: [], agents: [], tasks: [], messages: [], events: [], channels: [], projects: [], collaborations: [], queuedMessages: [], approvalRequests: [], approvalRules: [], settings: { accent: '#3978d4', theme: 'light', interfaceScale: 100, showToolActivity: true, showReasoningSummaries: true, sendWithEnter: false, sidebarView: 'standard' },
      subagentTranscripts: { retained: [{ id: 'transcript-old', taskId, role: 'assistant', text: 'old retained transcript', createdAt: 1 }], removed: [{ id: 'transcript-remove', taskId, role: 'assistant', text: 'must disappear', createdAt: 1 }] },
      channels: [{ id: 'kept-channel', name: 'old name', description: '', agentIds: [], messages: [{ id: 'channel-history', role: 'user', agentId: null, text: 'keep channel message', createdAt: 1, taskId }] }, { id: 'removed-channel', name: 'remove me', description: '', agentIds: [], messages: [] }] });
    const currentSnapshot = () => ({ ...emptySnapshot(), messages: messages.slice(-64) });
    const metadata = () => ({ ...currentSnapshot(), messages: [], channels: [{ ...emptySnapshot().channels[0], name: 'fresh name', messages: [] }], subagentTranscripts: {} });
    const state = { snapshot: currentSnapshot(), pageCalls: 0, resetReads: 0 };
    window.__syncServer = {
      state,
      delayNextPage() { delayPage = true; },
      releasePage() { releasePage?.(); },
      pushDelta() {
        const fromRevision = revision;
        revision = `rev-${Number(revision.split('-')[1]) + 1}`;
        const updated = { ...messages.find(message => message.id === 'm100'), text: 'message-100-updated-live' };
        messages[messages.findIndex(message => message.id === 'm100')] = updated;
        messages.splice(messages.findIndex(message => message.id === 'm099'), 1);
        delta = { revision, snapshot: null, delta: { fromRevision, metadata: metadata(), messages: [updated], removedMessageIds: ['m099'], retainedChannelIds: ['kept-channel'], retainedSubagentTranscriptIds: ['retained'] } };
        state.snapshot = currentSnapshot();
      },
      forceGap() { staleGap = true; revision = `rev-${Number(revision.split('-')[1]) + 1}`; state.snapshot = currentSnapshot(); },
    };
    window.__MONITTER_TEST_BRIDGE__ = {
      async invoke(command) { if (command === 'get_snapshot') return state.snapshot; throw Error(`Unexpected legacy command ${command}`); },
      async getUiDelta(requestRevision) {
        if (requestRevision === undefined) { state.resetReads += 1; delta = null; return { revision, snapshot: state.snapshot, delta: null }; }
        if (staleGap) { staleGap = false; return { revision, snapshot: null, delta: { fromRevision: 'stale-base', metadata: metadata(), messages: [], removedMessageIds: [], retainedChannelIds: [], retainedSubagentTranscriptIds: [] } }; }
        if (delta && delta.delta.fromRevision === requestRevision) { const next = delta; delta = null; return next; }
        if (requestRevision !== revision) return { revision, snapshot: state.snapshot, delta: null };
        return { revision, snapshot: null, delta: null };
      },
      async getTaskMessages(requestTaskId, beforeId, limit = 16) {
        state.pageCalls += 1;
        const pageRevision = revision;
        if (delayPage) { delayPage = false; pendingPageResolve = true; await pageGate; }
        if (requestTaskId !== taskId) throw Error('wrong task');
        const end = beforeId ? messages.findIndex(message => message.id === beforeId) : messages.length;
        const safeEnd = end < 0 ? messages.length : end;
        const start = Math.max(0, safeEnd - Math.min(100, Math.max(1, limit)));
        const pageMessages = messages.slice(start, safeEnd);
        if (pendingPageResolve) { pendingPageResolve = false; }
        return { revision: pageRevision, messages: pageMessages, nextBeforeId: start > 0 ? pageMessages[0]?.id ?? null : null };
      },
      async listen() { return () => {}; },
    };
  });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(`http://127.0.0.1:${address.port}/`);
  await expect(page.locator('[data-message-id="m100"]')).toBeVisible();
  const viewport = page.locator('.messages');
  await page.waitForFunction(() => !!document.querySelector('.messages') && document.querySelector('.messages').scrollHeight > document.querySelector('.messages').clientHeight);
  await expect.poll(() => viewport.evaluate(node => node.scrollHeight - node.clientHeight - node.scrollTop)).toBeLessThanOrEqual(3);
  await viewport.evaluate(node => {
    node.scrollTop = node.scrollHeight - node.clientHeight - 280;
    node.dispatchEvent(new WheelEvent('wheel', { deltaY: -80, bubbles: true }));
    node.dispatchEvent(new Event('scroll'));
  });
  await expect(page.getByRole('button', { name: 'Jump to latest message' })).toBeVisible();
  const heldText = await page.locator('[data-message-id="m100"] [data-message-text]').textContent();
  await page.evaluate(() => window.__syncServer.delayNextPage());
  await page.getByRole('button', { name: 'Load earlier messages' }).click();
  await expect.poll(() => page.evaluate(() => window.__syncServer.state.pageCalls)).toBe(1);
  await page.evaluate(() => window.__syncServer.pushDelta());
  await page.evaluate(() => window.__syncQA.refresh());
  await page.evaluate(() => window.__syncServer.releasePage());
  await expect(page.locator('[data-message-id="m021"]')).toBeVisible();
  assert.equal(await page.locator('[data-message-id="m100"] [data-message-text]').textContent(), heldText, 'detached reader freezes live delta text');
  assert.equal(await page.locator('[data-message-id="m099"]').count(), 1, 'detached reader freezes live deletion');
  const anchorBefore = await viewport.evaluate(node => {
    const top = node.getBoundingClientRect().top;
    const row = [...node.querySelectorAll('[data-message-id]')].find(item => item.getBoundingClientRect().bottom > top + 10);
    return row && { id: row.getAttribute('data-message-id'), top: row.getBoundingClientRect().top };
  });
  assert.ok(anchorBefore, 'the detached viewport has a visible stable row');
  await page.evaluate(value => { window.__syncAnchorId = value.id; }, anchorBefore);
  await page.getByRole('button', { name: 'Load earlier messages' }).click();
  await expect(page.locator('[data-message-id="m005"]')).toBeVisible();
  const restoredAnchorTop = await viewport.evaluate(node => node.querySelector(`[data-message-id="${window.__syncAnchorId}"]`)?.getBoundingClientRect().top ?? null);
  assert.ok(anchorBefore && restoredAnchorTop !== null, 'page prepend retains the visible anchor row');
  assert.ok(Math.abs(restoredAnchorTop - anchorBefore.top) < 4, `prepend moves the reader anchor only ${Math.abs(restoredAnchorTop - anchorBefore.top)}px`);
  const detachedGap = await viewport.evaluate(node => node.scrollHeight - node.clientHeight - node.scrollTop);
  assert.ok(detachedGap > 40, 'loading history while detached does not jump to the bottom');
  await page.getByRole('button', { name: 'Jump to latest message' }).click();
  await expect(page.locator('[data-message-id="m100"] [data-message-text]')).toHaveText('message-100-updated-live');
  await expect(page.locator('[data-message-id="m099"]')).toHaveCount(0);
  const retained = await page.evaluate(() => window.__syncQA.state().snapshot);
  assert.equal(retained.channels[0].name, 'fresh name');
  assert.deepEqual(retained.channels[0].messages.map(message => message.id), ['channel-history']);
  assert.deepEqual(Object.keys(retained.subagentTranscripts).sort(), ['retained']);
  assert.equal(retained.subagentTranscripts.retained[0].id, 'transcript-old');

  await page.evaluate(async () => { window.__syncServer.forceGap(); await window.__syncQA.refresh(); });
  const reset = await page.evaluate(() => ({ state: window.__syncQA.state(), resetReads: window.__syncServer.state.resetReads }));
  assert.ok(reset.resetReads >= 2, 'stale fromRevision forces an unbased full snapshot reset');
  assert.equal(reset.state.snapshot.channels.some(channel => channel.id === 'removed-channel'), false, 'reset removes deleted channels');
  assert.deepEqual(errors, [], `browser errors: ${errors.join('; ')}`);
  console.log('UI sync browser fixture: delta/page race, exact revision recovery, retained histories, frozen reader, and anchored pagination passed.');
  await context.close();
} finally {
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
