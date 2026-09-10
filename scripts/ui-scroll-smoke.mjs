import { chromium, expect } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';

// Start the development server first. The injected bridge is browser-QA-only.
const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18420';
const output = 'verification/ui-scroll-results.json';
mkdirSync('verification', { recursive: true });

const browser = await chromium.launch({ headless: true });
const passed = [];
const pageErrors = [];
let page;

const longText = (label) => `${label}. ${'A deliberately long Monitter conversation entry stays in the message viewport. '.repeat(18)}`;

try {
  page = await browser.newPage({ viewport: { width: 1120, height: 680 } });
  page.on('pageerror', error => pageErrors.push(error.message));
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.goto(url);
  await expect(page.getByRole('button', { name: 'New task', exact: true }).first()).toBeVisible({ timeout: 30000 });

  await page.evaluate(source => { window.__scrollLongText = Function(`return (${source})`)(); }, longText.toString());
  const ids = await page.evaluate(() => {
    const qa = window.__MONITTER_QA__;
    const state = qa.snapshot();
    const now = Date.now();
    const task = (id, title, offset) => ({
      id, agentId: 'atlas', title, nativeSessionId: null, status: 'idle', archived: false,
      createdAt: now + offset, updatedAt: now + offset, parentTaskId: null, channelId: null,
      hostId: 'local', cwd: '/tmp/monitter-ui-test', provider: 'codex', model: '', sandbox: 'read-only',
    });
    state.tasks = [task('scroll-chat-a', 'Scroll chat A', 0), task('scroll-chat-b', 'Scroll chat B', 1)];
    state.messages = [];
    for (const taskId of ['scroll-chat-a', 'scroll-chat-b']) {
      for (let i = 0; i < 16; i++) state.messages.push({
        id: `${taskId}-${i}`, taskId, role: i % 3 === 0 ? 'user' : 'assistant',
        text: window.__scrollLongText(`${taskId} initial message ${i}`), createdAt: now + i,
      });
    }
    state.channels = [{
      id: 'scroll-channel', name: 'Scroll channel', description: 'Browser scroll fixture.', agentIds: ['atlas'],
      messages: Array.from({ length: 16 }, (_, i) => ({
        id: `scroll-channel-${i}`, role: i % 3 === 0 ? 'user' : 'assistant', agentId: i % 3 === 0 ? null : 'atlas',
        taskId: null, text: window.__scrollLongText(`channel initial message ${i}`), createdAt: now + i,
      })),
    }];
    qa.setSnapshot(state);
    return { chatA: 'scroll-chat-a', chatB: 'scroll-chat-b', channel: 'scroll-channel' };
  });

  const messages = page.locator('.messages');
  const jump = page.getByRole('button', { name: 'Jump to latest message', exact: true });
  const metrics = () => messages.evaluate(el => ({ top: el.scrollTop, height: el.clientHeight, total: el.scrollHeight }));
  const atBottom = async () => expect.poll(async () => {
    const { top, height, total } = await metrics();
    return total - height - top;
  }).toBeLessThanOrEqual(3);
  const assertDocumentDoesNotScroll = async () => {
    const dimensions = await page.evaluate(() => ({
      rootHeight: document.documentElement.scrollHeight, bodyHeight: document.body.scrollHeight,
      viewportHeight: innerHeight, rootTop: document.documentElement.scrollTop, bodyTop: document.body.scrollTop,
    }));
    expect(dimensions.rootHeight).toBeLessThanOrEqual(dimensions.viewportHeight);
    expect(dimensions.bodyHeight).toBeLessThanOrEqual(dimensions.viewportHeight);
    expect(dimensions.rootTop).toBe(0);
    expect(dimensions.bodyTop).toBe(0);
  };
  const scrollUp = async () => {
    await messages.hover();
    const { total, height } = await metrics();
    await page.mouse.wheel(0, -(total + height));
    await expect.poll(async () => (await metrics()).top).toBeLessThan(40);
  };
  const addChatMessage = taskId => page.evaluate(taskId => {
    const qa = window.__MONITTER_QA__, state = qa.snapshot();
    state.messages.push({ id: crypto.randomUUID(), taskId, role: 'assistant', text: window.__scrollLongText('incoming chat message'), createdAt: Date.now() });
    qa.setSnapshot(state);
  }, taskId);
  const addChannelMessage = () => page.evaluate(() => {
    const qa = window.__MONITTER_QA__, state = qa.snapshot();
    state.channels.find(channel => channel.id === 'scroll-channel').messages.push({
      id: crypto.randomUUID(), role: 'assistant', agentId: 'atlas', taskId: null,
      text: window.__scrollLongText('incoming channel message'), createdAt: Date.now(),
    });
    qa.setSnapshot(state);
  });
  const expandChatMessage = taskId => page.evaluate(taskId => {
    const qa = window.__MONITTER_QA__, state = qa.snapshot();
    const message = state.messages.filter(entry => entry.taskId === taskId).at(-1);
    message.text += ` ${window.__scrollLongText('late rendered content growth').repeat(3)}`;
    qa.setSnapshot(state);
  }, taskId);
  const waitForHeightGrowth = async beforeHeight => {
    await expect.poll(async () => (await metrics()).total).toBeGreaterThan(beforeHeight + 8);
  };

  const checkViewportBehaviour = async ({ open, incoming, label }) => {
    await open();
    await atBottom();
    await assertDocumentDoesNotScroll();

    await scrollUp();
    await expect(jump).toBeVisible();
    await jump.click();
    await atBottom();

    let beforeHeight = (await metrics()).total;
    await incoming();
    await waitForHeightGrowth(beforeHeight);
    await atBottom();

    await scrollUp();
    const before = await metrics();
    await incoming();
    await waitForHeightGrowth(before.total);
    await expect.poll(async () => (await metrics()).top).toBeGreaterThanOrEqual(before.top - 2);
    const after = await metrics();
    expect(after.top).toBeLessThanOrEqual(before.top + 2);
    await expect(jump).toBeVisible();
    await assertDocumentDoesNotScroll();
    passed.push(`${label}: activate at bottom / jump / follow / preserve history`);
  };

  const openChatA = async () => page.getByRole('button', { name: /Scroll chat A/ }).first().click();
  const openChatB = async () => page.getByRole('button', { name: /Scroll chat B/ }).first().click();
  const openChannel = async () => page.getByRole('button', { name: /Scroll channel/ }).first().click();

  await checkViewportBehaviour({ open: openChatA, incoming: () => addChatMessage(ids.chatA), label: 'chat' });
  // Selecting an already active chat is also an explicit request to see its latest message.
  await openChatA();
  await atBottom();
  passed.push('chat: reselecting the active chat opens at latest');
  await checkViewportBehaviour({ open: openChannel, incoming: addChannelMessage, label: 'channel' });

  // A stream can make an existing markdown block taller after it has rendered.
  // ResizeObserver must retain the same follow-or-preserve decision as a new item.
  await openChatA();
  await atBottom();
  let lateBefore = await metrics();
  await expandChatMessage(ids.chatA);
  await waitForHeightGrowth(lateBefore.total);
  await atBottom();
  await scrollUp();
  lateBefore = await metrics();
  await expandChatMessage(ids.chatA);
  await waitForHeightGrowth(lateBefore.total);
  const lateAfter = await metrics();
  expect(lateAfter.top).toBeGreaterThanOrEqual(lateBefore.top - 2);
  expect(lateAfter.top).toBeLessThanOrEqual(lateBefore.top + 2);
  await expect(jump).toBeVisible();
  passed.push('chat: late growth follows only when already near latest');

  await openChatA();
  await atBottom();
  await page.setViewportSize({ width: 1120, height: 560 });
  await atBottom();
  await assertDocumentDoesNotScroll();
  await page.setViewportSize({ width: 1120, height: 680 });
  await atBottom();
  passed.push('chat: viewport resize retains latest position');

  // A successful send is a user action and always brings the still-active conversation to its latest entry.
  await openChatA();
  await scrollUp();
  await page.getByLabel('Task message', { exact: true }).fill('Send chat scroll regression message');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'sendMessage').length)).toBeGreaterThan(0);
  await atBottom();
  await page.evaluate(taskId => {
    const qa = window.__MONITTER_QA__, state = qa.snapshot();
    state.tasks.find(task => task.id === taskId).status = 'idle';
    qa.setSnapshot(state);
  }, ids.chatA);
  passed.push('chat: successful send returns the active conversation to latest');

  await openChannel();
  await page.locator('.recipient-picker').getByRole('button', { name: 'Atlas', exact: true }).click();
  await scrollUp();
  await page.getByLabel('Channel message', { exact: true }).fill('Send channel scroll regression message');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'sendChannelMessage').length)).toBeGreaterThan(0);
  await atBottom();
  passed.push('channel: successful send returns the active conversation to latest');

  // Delay the IPC acknowledgement, navigate to another chat, and prove it does not steal that chat's scroll position.
  await openChatA();
  await page.evaluate(() => {
    const original = window.__MONITTER_BRIDGE__.sendMessage;
    let release;
    const pending = new Promise(resolve => { release = resolve; });
    window.__MONITTER_QA__.releaseScrollSend = release;
    window.__MONITTER_BRIDGE__.sendMessage = async (...args) => {
      window.__MONITTER_BRIDGE__.sendMessage = original;
      await pending;
      return original(...args);
    };
  });
  await page.getByLabel('Task message', { exact: true }).fill('Delayed send must stay scoped');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await openChatB();
  await atBottom();
  await scrollUp();
  const otherBefore = await metrics();
  await page.evaluate(() => window.__MONITTER_QA__.releaseScrollSend());
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks.find(task => task.id === 'scroll-chat-a').status)).toBe('running');
  const otherAfter = await metrics();
  expect(otherAfter.top).toBeGreaterThanOrEqual(otherBefore.top - 2);
  expect(otherAfter.top).toBeLessThanOrEqual(otherBefore.top + 2);
  await assertDocumentDoesNotScroll();
  passed.push('delayed chat send completion does not scroll a different active chat');

  expect(pageErrors).toEqual([]);
  const report = { completedAt: new Date().toISOString(), passed, pageErrors, output };
  writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`);
  console.log(JSON.stringify(report, null, 2));
} catch (error) {
  const failure = { completedAt: new Date().toISOString(), passed, pageErrors, error: String(error) };
  writeFileSync(output, `${JSON.stringify(failure, null, 2)}\n`);
  if (page) await page.screenshot({ path: 'verification/ui-scroll-failure.png' });
  throw error;
} finally {
  await browser.close();
}
