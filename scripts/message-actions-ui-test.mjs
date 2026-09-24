import { chromium, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';

const browser = await chromium.launch({ headless: true, executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' });
const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, permissions: ['clipboard-read', 'clipboard-write'] });
const page = await context.newPage();
const errors = [];
page.on('pageerror', error => errors.push(error.message));

try {
  await page.addInitScript({ content: readFileSync('scripts/ui-fixture.js', 'utf8') + `
    const qa = window.__MONITTER_QA__, snapshot = qa.snapshot(), now = Date.now();
    snapshot.tasks.push({ id:'action-chat', agentId:'atlas', title:'Message actions', nativeSessionId:null, status:'idle', archived:false, createdAt:now, updatedAt:now, parentTaskId:null, channelId:null, projectId:null, hostId:'local', cwd:'/tmp', provider:'codex', model:'qa-balanced', sandbox:'read-only' });
    snapshot.messages.push(
      { id:'first-user', taskId:'action-chat', role:'user', text:'First request', createdAt:now-3000, attachments:[] },
      { id:'first-agent', taskId:'action-chat', role:'assistant', text:'First answer', createdAt:now-2000, attachments:[] },
      { id:'later-user', taskId:'action-chat', role:'user', text:'Later request', createdAt:now-1000, attachments:[] },
    );
    snapshot.events.push({ id:'first-tool', taskId:'action-chat', kind:'tool', title:'Run command', detail:'{"type":"command_execution","command":"pwd"}', createdAt:now-2500 });
    qa.setSnapshot(snapshot);
    window.__MONITTER_BRIDGE__.forkTask = async input => {
      qa.calls.push({ method:'forkTask', args:input });
      const state = qa.snapshot(), source = state.tasks.find(task => task.id === input.sourceTaskId);
      const cutoff = state.messages.findIndex(message => message.id === input.throughMessageId && message.taskId === source.id);
      if (cutoff < 0) throw Error('Selected message was not found.');
      const target = { ...source, id:crypto.randomUUID(), title:source.title+' · fork', createdAt:Date.now(), updatedAt:Date.now(), parentTaskId:source.id, nativeSessionId:null, status:'idle', agentId:input.agentId, modelSettings:input.modelSettings, model:input.modelSettings?.model || source.model };
      state.tasks.push(target);
      state.messages.push(...state.messages.slice(0, cutoff+1).filter(message => message.taskId === source.id && ['user','assistant'].includes(message.role)).map(message => ({ ...message, id:crypto.randomUUID(), taskId:target.id, attachments:[] })));
      state.messages.push({ id:crypto.randomUUID(), taskId:target.id, role:'system', text:'[Monitter fork] '+source.title, createdAt:Date.now(), attachments:[] });
      qa.setSnapshot(state);
      return target;
    };
  ` });
  await page.goto('http://127.0.0.1:18422/monitter-app-ui/', { timeout: 60000 });
  await page.locator('.sidebar .task-select').filter({ hasText:'Message actions' }).click();
  const answer = page.locator('.message').filter({ hasText:'First answer' });
  await expect(answer).toBeVisible();
  const messageMargins = await answer.evaluate(node => {
    const style = getComputedStyle(node);
    return { top:style.marginTop, bottom:style.marginBottom };
  });
  expect(messageMargins).toEqual({ top:'12px', bottom:'12px' });
  const tree = page.locator('.messages .process-tree');
  await expect(tree).toBeVisible();
  const edges = await tree.evaluate(node => {
    const bubble = node.closest('.transcript-virtual-list').querySelector('.message:not(.user) .message-bubble').getBoundingClientRect();
    const rect = node.getBoundingClientRect();
    const icon = node.querySelector(':scope > summary svg').getBoundingClientRect();
    return { left:Math.abs(rect.left - bubble.left), right:Math.abs(rect.right - bubble.right), inset:Math.abs(icon.left - rect.left), opacity:getComputedStyle(node).opacity };
  });
  expect(edges.left).toBeLessThanOrEqual(2);
  expect(edges.right).toBeLessThanOrEqual(2);
  expect(edges.inset).toBeLessThanOrEqual(2);
  expect(edges.opacity).toBe('0.6');
  await tree.hover();
  await expect.poll(() => tree.evaluate(node => getComputedStyle(node).opacity)).toBe('1');
  const positions = await answer.evaluate(node => {
    const author = node.querySelector('.message-author').getBoundingClientRect();
    const actions = node.querySelector('.message-actions').getBoundingClientRect();
    const time = node.querySelector('time').getBoundingClientRect();
    return { authorRight:author.right, actionsLeft:actions.left, actionsRight:actions.right, timeLeft:time.left };
  });
  expect(positions.actionsLeft).toBeGreaterThanOrEqual(positions.authorRight);
  expect(positions.timeLeft).toBeGreaterThan(positions.actionsRight);

  await answer.getByRole('button', { name:'Copy message text' }).click();
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe('First answer');
  await answer.getByRole('button', { name:'Attach message to reply' }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.calls.find(call => call.method === 'storeAttachment')?.args?.filename)).toBe('Reply to Atlas.md');
  expect(await page.evaluate(() => atob(window.__MONITTER_QA__.calls.find(call => call.method === 'storeAttachment').args.dataBase64))).toBe('First answer');
  await expect(page.locator('.composer').getByText('Reply to Atlas.md')).toBeVisible();
  await expect(page.getByLabel('Task message', { exact:true })).toBeFocused();

  await answer.getByRole('button', { name:'Fork chat through this message' }).click();
  await expect(page.getByRole('dialog', { name:'Fork this chat' })).toBeVisible();
  await page.getByRole('dialog', { name:'Fork this chat' }).getByRole('combobox', { name:'Fork model' }).selectOption('qa-simple');
  await page.getByRole('dialog', { name:'Fork this chat' }).getByRole('button', { name:'Create fork' }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.calls.find(call => call.method === 'forkTask')?.args?.throughMessageId)).toBe('first-agent');
  const fork = await page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks.find(task => task.parentTaskId === 'action-chat'));
  expect(fork).toMatchObject({ status:'idle', model:'qa-simple', modelSettings:{ model:'qa-simple' } });
  const history = await page.evaluate(taskId => window.__MONITTER_QA__.snapshot().messages.filter(message => message.taskId === taskId && ['user','assistant'].includes(message.role)).map(message => message.text), fork.id);
  expect(history).toEqual(['First request', 'First answer']);
  await expect(page.locator('.tabs').getByRole('button', { name:'Message actions · fork', exact:true })).toBeVisible();
  const bridgePage = await context.newPage();
  await bridgePage.goto('http://127.0.0.1:18422/monitter-app-ui/__monitter_dev__');
  const attachmentArgs = await bridgePage.evaluate(async () => {
    let captured;
    window.__MONITTER_TEST_BRIDGE__ = { invoke: async (command, args) => {
      if (command === 'store_attachment') captured = args;
      return {};
    } };
    const { getBridge } = await import('/monitter-app-ui/src/lib/bridge.ts');
    await getBridge().storeAttachment({ taskId:'test-task' }, { filename:'Reply.md', mimeType:'text/markdown', dataBase64:'dGV4dA==' }, null, 'source-id');
    return captured;
  });
  expect(attachmentArgs).not.toHaveProperty('previewDataUrl');
  expect(attachmentArgs).toHaveProperty('sourceId', 'source-id');
  expect(errors).toEqual([]);
  console.log('Message actions: timestamp layout, copy, reply attachment, and model-selected fork passed.');
} finally {
  await browser.close();
}
