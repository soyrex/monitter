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
    const channelMessages = Array.from({ length: 100 }, (_, index) => ({ id: `c${String(index + 1).padStart(3, '0')}`, role: index % 2 ? 'assistant' : 'user', agentId: null, taskId: null, text: `channel-${index + 1}`, createdAt: index + 1 }));
    const channelSeed = { id: 'channel-history', role: 'user', agentId: null, taskId: taskId, text: 'keep channel message', createdAt: 0 };
    let revision = 'rev-1'; let delta = null; let staleGap = false; let delayPage = false; let releasePage; let pendingPageResolve; let delayChannelPage = false; let releaseChannelPage; let removedChannel = false; let removedTranscript = false;
    const pageGate = new Promise(resolve => { releasePage = resolve; });
    const emptySnapshot = () => ({ hosts: [], agents: [], tasks: [], messages: [], events: [], channels: [], projects: [], collaborations: [], queuedMessages: [], approvalRequests: [], approvalRules: [], settings: { accent: '#3978d4', theme: 'light', interfaceScale: 100, showToolActivity: true, showReasoningSummaries: true, sendWithEnter: false, sidebarView: 'standard' },
      subagentTranscripts: { retained: [{ id: 'transcript-old', taskId, role: 'assistant', text: 'old retained transcript', createdAt: 1 }], removed: [{ id: 'transcript-remove', taskId, role: 'assistant', text: 'must disappear', createdAt: 1 }] },
      channels: [{ id: 'kept-channel', name: 'old name', description: '', agentIds: [], messages: [channelSeed, ...channelMessages].slice(-64) }, { id: 'other-channel', name: 'other channel', description: '', agentIds: [], messages: Array.from({ length: 80 }, (_, index) => ({ id: `d${String(index + 1).padStart(3, '0')}`, role: 'assistant', agentId: null, taskId: null, text: `other-${index + 1}`, createdAt: index + 1 })).slice(-64) }, { id: 'removed-channel', name: 'remove me', description: '', agentIds: [], messages: [] }] });
    const currentSnapshot = () => ({ ...emptySnapshot(), messages: messages.slice(-64),
      channels: emptySnapshot().channels.filter(channel => !(removedChannel && channel.id === 'removed-channel')),
      subagentTranscripts: removedTranscript ? { retained: emptySnapshot().subagentTranscripts.retained } : emptySnapshot().subagentTranscripts });
    const metadata = () => ({ ...currentSnapshot(), messages: [], channels: emptySnapshot().channels.filter(channel => !(removedChannel && channel.id === 'removed-channel')).map(channel => ({ ...channel, name: channel.id === 'kept-channel' ? 'fresh name' : channel.name, messages: [] })), subagentTranscripts: {} });
    const state = { snapshot: currentSnapshot(), pageCalls: 0, channelPageCalls: 0, resetReads: 0 };
    window.__syncServer = {
      state,
      delayNextPage() { delayPage = true; },
      releasePage() { releasePage?.(); },
      delayNextChannelPage() { delayChannelPage = true; },
      releaseChannelPage() { releaseChannelPage?.(); },
      pushDelta() {
        const fromRevision = revision;
        revision = `rev-${Number(revision.split('-')[1]) + 1}`;
        const updated = { ...messages.find(message => message.id === 'm100'), text: 'message-100-updated-live' };
        messages[messages.findIndex(message => message.id === 'm100')] = updated;
        messages.splice(messages.findIndex(message => message.id === 'm099'), 1);
        removedChannel = true; removedTranscript = true;
        delta = { revision, snapshot: null, delta: { fromRevision, metadata: metadata(), messages: [updated], removedMessageIds: ['m099'], retainedChannelIds: ['kept-channel', 'other-channel'], retainedSubagentTranscriptIds: ['retained'] } };
        state.snapshot = currentSnapshot();
      },
      pushArchiveDelta() {
        const fromRevision = revision;
        revision = `rev-${Number(revision.split('-')[1]) + 1}`;
        const updated = { ...messages.find(message => message.id === 'm022'), text: 'message-22-updated-live' };
        messages[messages.findIndex(message => message.id === 'm022')] = updated;
        messages.splice(messages.findIndex(message => message.id === 'm021'), 1);
        delta = { revision, snapshot: null, delta: { fromRevision, metadata: metadata(), messages: [updated], removedMessageIds: ['m021'], retainedChannelIds: ['kept-channel', 'other-channel'], retainedSubagentTranscriptIds: ['retained'] } };
        state.snapshot = currentSnapshot();
      },
      pushChannelDelta() {
        const fromRevision = revision;
        revision = `rev-${Number(revision.split('-')[1]) + 1}`;
        const updated = { ...channelMessages.find(message => message.id === 'c022'), text: 'channel-22-updated-live' };
        channelMessages[channelMessages.findIndex(message => message.id === 'c022')] = updated;
        channelMessages.splice(channelMessages.findIndex(message => message.id === 'c021'), 1);
        delta = { revision, snapshot: null, delta: { fromRevision, metadata: metadata(), messages: [], removedMessageIds: [], retainedChannelIds: ['other-channel'], channelMessageChanges: [{ channelId: 'kept-channel', messages: [updated], removedMessageIds: ['c021'], reset: false }], retainedSubagentTranscriptIds: ['retained'] } };
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
      async getChannelMessages(requestChannelId, beforeId, limit = 16) {
        state.channelPageCalls += 1;
        const pageRevision = revision;
        if (!['kept-channel', 'other-channel'].includes(requestChannelId)) throw Error('wrong channel');
        if (delayChannelPage) { delayChannelPage = false; await new Promise(resolve => { releaseChannelPage = resolve; }); }
        const all = requestChannelId === 'kept-channel' ? [channelSeed, ...channelMessages] : Array.from({ length: 80 }, (_, index) => ({ id: `d${String(index + 1).padStart(3, '0')}`, role: 'assistant', agentId: null, taskId: null, text: `other-${index + 1}`, createdAt: index + 1 }));
        const end = beforeId ? all.findIndex(message => message.id === beforeId) : all.length;
        if (end < 0) throw Error('unknown channel cursor');
        const start = Math.max(0, end - Math.min(100, Math.max(1, limit)));
        const pageMessages = all.slice(start, end);
        return { channelId: requestChannelId, revision: pageRevision, messages: pageMessages, nextBeforeId: start > 0 ? pageMessages[0]?.id ?? null : null };
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
  const heldText = await page.evaluate(() => window.__syncQA.state().displayMessages.find(message => message.id === 'm100').text);
  await page.evaluate(() => window.__syncServer.delayNextPage());
  await page.getByRole('button', { name: 'Load earlier messages' }).click();
  await expect.poll(() => page.evaluate(() => window.__syncServer.state.pageCalls)).toBe(1);
  await page.evaluate(() => window.__syncServer.pushDelta());
  await page.evaluate(() => window.__syncQA.refresh());
  await page.evaluate(() => window.__syncServer.releasePage());
  await expect.poll(() => page.evaluate(() => window.__syncQA.state().messages.some(message => message.id === 'm021'))).toBe(true);
  const detached = await page.evaluate(() => window.__syncQA.state());
  assert.equal(detached.displayMessages.find(message => message.id === 'm100').text, heldText, 'detached reader freezes live delta text');
  assert.equal(detached.displayMessages.some(message => message.id === 'm099'), true, 'detached reader freezes live deletion');
  assert.equal(detached.snapshot.messages.some(message => message.id === 'm099'), false, 'cached snapshot applies the deletion while held');
  assert.equal(await page.evaluate(() => window.__syncServer.state.pageCalls), 2, 'a page read racing a delta retries once against the current revision');
  const anchorBefore = await viewport.evaluate(node => {
    const top = node.getBoundingClientRect().top;
    const row = [...node.querySelectorAll('[data-message-id]')].find(item => item.getBoundingClientRect().bottom > top + 10);
    return row && { id: row.getAttribute('data-message-id'), top: row.getBoundingClientRect().top };
  });
  assert.ok(anchorBefore, 'the detached viewport has a visible stable row');
  await page.evaluate(value => { window.__syncAnchorId = value.id; }, anchorBefore);
  await page.getByRole('button', { name: 'Load earlier messages' }).click();
  await expect.poll(() => page.evaluate(() => window.__syncQA.state().messages.some(message => message.id === 'm005'))).toBe(true);
  await expect.poll(() => page.evaluate(() => window.__syncQA.state().loading)).toBe(false);
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  const restoredAnchorTop = await viewport.evaluate(node => node.querySelector(`[data-message-id="${window.__syncAnchorId}"]`)?.getBoundingClientRect().top ?? null);
  assert.ok(anchorBefore && restoredAnchorTop !== null, 'page prepend retains the visible anchor row');
  assert.ok(Math.abs(restoredAnchorTop - anchorBefore.top) < 4, `prepend moves the reader anchor only ${Math.abs(restoredAnchorTop - anchorBefore.top)}px`);
  const detachedGap = await viewport.evaluate(node => node.scrollHeight - node.clientHeight - node.scrollTop);
  assert.ok(detachedGap > 40, 'loading history while detached does not jump to the bottom');
  await page.evaluate(() => window.__syncServer.pushArchiveDelta());
  await page.evaluate(() => window.__syncQA.refresh());
  const staleOverlay = await page.evaluate(() => window.__syncQA.state());
  assert.equal(staleOverlay.messages.find(message => message.id === 'm021')?.text, 'message-21', 'an already-loaded deleted page row remains frozen while detached');
  assert.equal(staleOverlay.messages.find(message => message.id === 'm022')?.text, 'message-22', 'an already-loaded updated page row remains frozen while detached');
  await page.getByRole('button', { name: 'Jump to latest message' }).click();
  await expect.poll(() => page.evaluate(() => window.__syncQA.state().held)).toBe(false);
  const released = await page.evaluate(() => window.__syncQA.state());
  assert.equal(released.messages.some(message => message.id === 'm021'), false, 'releasing the reader does not resurrect a deleted older page row');
  assert.equal(released.messages.find(message => message.id === 'm022')?.text, 'message-22-updated-live', 'releasing the reader shows updated text from the authoritative snapshot');
  await expect(page.locator('[data-message-id="m100"] [data-message-text]')).toHaveText('message-100-updated-live');
  await expect(page.locator('[data-message-id="m099"]')).toHaveCount(0);
  const retained = await page.evaluate(() => window.__syncQA.state().snapshot);
  assert.equal(retained.channels[0].name, 'fresh name');
  assert.deepEqual(retained.channels[0].messages.map(message => message.id), Array.from({ length: 64 }, (_, index) => `c${String(index + 37).padStart(3, '0')}`));
  assert.deepEqual(Object.keys(retained.subagentTranscripts).sort(), ['retained']);
  assert.equal(retained.subagentTranscripts.retained[0].id, 'transcript-old');

  await page.evaluate(async () => { window.__syncServer.forceGap(); await window.__syncQA.refresh(); });
  const reset = await page.evaluate(() => ({ state: window.__syncQA.state(), resetReads: window.__syncServer.state.resetReads }));
  assert.ok(reset.resetReads >= 2, 'stale fromRevision forces an unbased full snapshot reset');
  assert.equal(reset.state.snapshot.channels.some(channel => channel.id === 'removed-channel'), false, 'reset removes deleted channels');
  assert.deepEqual(reset.state.snapshot.channels[0].messages.map(message => message.id), Array.from({ length: 64 }, (_, index) => `c${String(index + 37).padStart(3, '0')}`), 'initial/reset channel snapshot remains bounded to its newest 64 messages');

  await page.evaluate(() => window.__syncQA.showChannel());
  await expect(page.locator('[data-channel-message-id="c100"]')).toBeVisible();
  const channelViewport = page.locator('.messages');
  await page.waitForFunction(() => {
    const node = document.querySelector('.messages');
    return !!node && node.scrollHeight > node.clientHeight && node.scrollHeight - node.clientHeight - node.scrollTop <= 3;
  });
  await page.evaluate(() => window.__syncServer.delayNextChannelPage());
  await page.getByRole('button', { name: 'Load earlier channel messages' }).click();
  await expect.poll(() => page.evaluate(() => window.__syncServer.state.channelPageCalls)).toBe(1);
  await expect.poll(() => page.evaluate(() => window.__syncQA.channelHeld())).toBe(true);
  const channelAnchor = await channelViewport.evaluate(node => {
    const top = node.getBoundingClientRect().top;
    const row = [...node.querySelectorAll('[data-item-key]')].find(item => item.getBoundingClientRect().bottom > top + 10);
    return row && { key: row.getAttribute('data-item-key'), offset: row.getBoundingClientRect().top - top };
  });
  assert.ok(channelAnchor, 'channel reader starts with a visible stable row');
  await page.evaluate(value => { window.__syncChannelAnchorKey = value.key; }, channelAnchor);
  await page.evaluate(() => window.__syncServer.releaseChannelPage());
  const channelHistoryButton = page.getByRole('button', { name: 'Load earlier channel messages' });
  await expect(channelHistoryButton).toHaveCount(0);
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  const restoredChannelAnchorOffset = await channelViewport.evaluate(node => {
    const row = node.querySelector(`[data-item-key="${window.__syncChannelAnchorKey}"]`);
    return row ? row.getBoundingClientRect().top - node.getBoundingClientRect().top : null;
  });
  assert.ok(restoredChannelAnchorOffset !== null, 'channel prepend keeps the captured anchor mounted');
  assert.ok(Math.abs(restoredChannelAnchorOffset - channelAnchor.offset) < 4, 'channel prepend preserves the detached reader anchor relative to the scroll viewport');
  await channelViewport.evaluate(node => { node.scrollTop = 850; node.dispatchEvent(new Event('scroll')); });
  await expect(page.locator('[data-channel-message-id="c021"] [data-message-text]')).toHaveText('channel-21');
  await page.evaluate(() => window.__syncServer.pushChannelDelta());
  await page.evaluate(() => window.__syncQA.refresh());
  await expect(page.locator('[data-channel-message-id="c021"] [data-message-text]')).toHaveText('channel-21');
  await expect(page.locator('[data-channel-message-id="c022"] [data-message-text]')).toHaveText('channel-22');
  const changedChannelCache = await page.evaluate(() => window.__syncQA.state().snapshot.channels.find(channel => channel.id === 'kept-channel').messages);
  assert.equal(changedChannelCache.some(message => message.id === 'c021'), false, 'channel delta removes a page-loaded history record from cache');
  assert.equal(changedChannelCache.find(message => message.id === 'c022')?.text, 'channel-22-updated-live', 'channel delta updates a page-loaded history record in cache');
  await page.getByRole('button', { name: 'Jump to latest message' }).click();
  await expect.poll(() => page.evaluate(() => window.__syncQA.channelHeld())).toBe(false);
  const releasedChannelMessages = await page.evaluate(() => window.__syncQA.state().snapshot.channels.find(channel => channel.id === 'kept-channel').messages);
  assert.equal(releasedChannelMessages.some(message => message.id === 'c021'), false, 'channel reader release does not resurrect deleted page history');
  assert.equal(releasedChannelMessages.find(message => message.id === 'c022')?.text, 'channel-22-updated-live', 'channel reader release shows updated page history');

  await page.evaluate(() => window.__syncQA.switchChannel('other-channel'));
  await expect(page.locator('[data-channel-message-id="d080"]')).toBeVisible();
  const otherChannelPager = page.getByRole('button', { name: 'Load earlier channel messages' });
  await expect(otherChannelPager).toBeVisible();
  await page.evaluate(() => window.__syncServer.delayNextChannelPage());
  await otherChannelPager.click();
  await expect.poll(() => page.evaluate(() => window.__syncServer.state.channelPageCalls)).toBe(2);
  await page.evaluate(() => window.__syncQA.switchChannel('kept-channel'));
  await page.evaluate(() => window.__syncServer.releaseChannelPage());
  assert.equal(await page.evaluate(() => window.__syncQA.currentChannel()), 'kept-channel');
  await expect(page.locator('[data-channel-message-id^="d"]')).toHaveCount(0);
  assert.deepEqual(errors, [], `browser errors: ${errors.join('; ')}`);
  console.log('UI sync browser fixture: task/channel delta paging, bounded channel reset, reader freeze/release, and anchored history passed.');
  await context.close();
} finally {
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
