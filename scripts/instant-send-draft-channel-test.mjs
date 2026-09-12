import { webkit, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';

const browser = await webkit.launch();
const page = await browser.newPage({ viewport: { width: 960, height: 760 } });
const errors = [];
page.on('pageerror', error => errors.push(error.message));

await page.addInitScript(readFileSync('scripts/ui-fixture.js', 'utf8') + `
  document.documentElement?.setAttribute('data-monitter-lan', '1');
  document.addEventListener('DOMContentLoaded', () => document.documentElement.setAttribute('data-monitter-lan', '1'));
  const qa = window.__MONITTER_QA__, state = qa.snapshot(), now = Date.now();
  state.tasks = [{id:'rapid',agentId:'atlas',title:'Rapid sends',nativeSessionId:null,status:'completed',archived:false,createdAt:now,updatedAt:now,parentTaskId:null,channelId:null,projectId:null,hostId:'local',cwd:'/tmp/monitter-ui-test',provider:'codex',model:'',sandbox:'read-only'}];
  state.channels = [{id:'instant-room',name:'Instant room',description:'',agentIds:['atlas'],messages:[]}];
  qa.setSnapshot(state);
  const originalCreate = window.__MONITTER_BRIDGE__.createTask;
  const originalSend = window.__MONITTER_BRIDGE__.sendMessage;
  const originalChannelSend = window.__MONITTER_BRIDGE__.sendChannelMessage;
  window.__draftCreate = null; window.__taskSends = []; window.__channelSends = [];
  window.__MONITTER_BRIDGE__.createTask = async input => await new Promise(resolve => { window.__draftCreate = () => originalCreate(input).then(resolve); });
  window.__MONITTER_BRIDGE__.sendMessage = async (...args) => await new Promise((resolve, reject) => window.__taskSends.push({args,resolve,reject}));
  window.__MONITTER_BRIDGE__.sendChannelMessage = async (...args) => await new Promise(resolve => window.__channelSends.push({args,resolve}));
  window.__resolveTaskSend = async index => {
    const pending = window.__taskSends[index], [taskId, text] = pending.args;
    const next = qa.snapshot();
    next.messages.push({id:'server-'+index,taskId,role:'user',text,createdAt:Date.now(),attachments:[]});
    qa.setSnapshot(next); pending.resolve(next);
  };
  window.__resolveChannelSend = async index => {
    const pending = window.__channelSends[index], [channelId, text] = pending.args;
    const next = qa.snapshot();
    next.channels.find(channel => channel.id === channelId).messages.push({id:'channel-'+index,role:'user',agentId:null,text,createdAt:Date.now(),taskId:null,attachments:[]});
    qa.setSnapshot(next); pending.resolve(next);
  };
`);

try {
  await page.route('**/api/access', route => route.fulfill({ json: { required: false } }));
  await page.goto(process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18440/');

  // A first message is visibly in the outbox before create_task resolves.
  await page.getByRole('button', { name: 'New chat', exact: true }).first().click();
  const draftComposer = page.getByRole('textbox', { name: 'Task message', exact: true });
  await draftComposer.fill('Draft appears before task creation');
  await page.getByRole('button', { name: 'Send task message', exact: true }).click();
  const draftBubble = page.locator('article.message').filter({ hasText: 'Draft appears before task creation' });
  await expect(draftBubble).toHaveCount(1);
  await expect(draftBubble).toContainText(/sending/i);
  await expect.poll(() => page.evaluate(() => Boolean(window.__draftCreate))).toBe(true);
  await page.evaluate(() => window.__draftCreate());
  await expect.poll(() => page.evaluate(() => window.__taskSends.length)).toBe(1);
  await page.evaluate(() => window.__resolveTaskSend(0));
  await expect(page.locator('article.message').filter({ hasText: 'Draft appears before task creation' })).toHaveCount(1);

  // A channel gets the same immediate echo, before its bridge promise resolves.
  await page.getByRole('complementary', { name: 'Agents and tasks' }).getByRole('button', { name: /Instant room/ }).click();
  const channelComposer = page.getByRole('textbox', { name: 'Channel message', exact: true });
  await channelComposer.fill('Channel instant echo');
  await page.locator('.recipient-picker').getByRole('button', { name: 'Atlas', exact: true }).click();
  await page.getByRole('button', { name: 'Send channel message', exact: true }).click();
  const channelBubble = page.locator('article.message').filter({ hasText: 'Channel instant echo' });
  await expect(channelBubble).toHaveCount(1);
  await expect(channelBubble).toContainText(/sending/i);
  await page.evaluate(() => window.__resolveChannelSend(0));
  await expect(channelBubble).toHaveCount(1);
  await expect(channelBubble).toContainText(/sent/i);

  // Identical rapid messages are reconciled one server message at a time.
  await page.locator('[data-task-id="rapid"] .task-select').click();
  const taskComposer = page.getByRole('textbox', { name: 'Task message', exact: true });
  for (let index = 0; index < 2; index += 1) {
    await taskComposer.fill('Same text twice');
    await page.getByRole('button', { name: 'Send task message', exact: true }).click();
  }
  await expect(page.locator('article.message').filter({ hasText: 'Same text twice' })).toHaveCount(2);
  await page.evaluate(() => window.__resolveTaskSend(1));
  await expect(page.locator('article.message').filter({ hasText: 'Same text twice' })).toHaveCount(2);
  await page.evaluate(() => window.__MONITTER_QA__.emit());
  await expect(page.locator('article.message').filter({ hasText: 'Same text twice' })).toHaveCount(2);
  await page.evaluate(() => window.__resolveTaskSend(2));
  await expect(page.locator('article.message').filter({ hasText: 'Same text twice' })).toHaveCount(2);

  // A rejected send stays singular; retry refreshes first, then creates one replacement request.
  await taskComposer.fill('Retry has one bubble');
  await page.getByRole('button', { name: 'Send task message', exact: true }).click();
  const retryBubble = page.locator('article.message').filter({ hasText: 'Retry has one bubble' });
  await page.evaluate(() => window.__taskSends[3].reject(new Error('lost response')));
  await expect(retryBubble).toContainText(/not confirmed/i);
  await retryBubble.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__taskSends.length)).toBe(5);
  await expect(retryBubble).toHaveCount(1);
  expect(errors).toEqual([]);
  console.log('Instant draft/channel outbox, one-to-one reconciliation and retry safety pass.');
} finally {
  await browser.close();
}
