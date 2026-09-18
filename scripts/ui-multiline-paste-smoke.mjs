import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { webkit, expect } from '@playwright/test';

const browser = await webkit.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
  await page.addInitScript({ content: readFileSync('scripts/ui-fixture.js', 'utf8') + `
    const qa = window.__MONITTER_QA__, state = qa.snapshot(), now = Date.now();
    state.tasks = [{ id:'multiline-task', agentId:'atlas', title:'Multiline QA', nativeSessionId:null,
      status:'idle', archived:false, createdAt:now, updatedAt:now, parentTaskId:null, channelId:null,
      projectId:null, hostId:'local', cwd:'/tmp/monitter-ui-test', provider:'codex', model:'', sandbox:'read-only' }];
    qa.setSnapshot(state);
  ` });
  await page.goto(process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18467', { waitUntil: 'domcontentloaded', timeout: 120_000 });
  await page.locator('.sidebar .task-select').click({ timeout: 240_000 });

  const composer = page.getByRole('textbox', { name: 'Task message', exact: true });
  const pasted = 'First line\nSecond line\n\nFourth line';
  const pastePrevented = await composer.evaluate((element, text) => {
    const data = new DataTransfer();
    data.setData('text/plain', text);
    data.items.add(new File(['attachment'], 'pasted.txt', { type: 'text/plain' }));
    const event = new ClipboardEvent('paste', { bubbles: true, cancelable: true, clipboardData: data });
    element.dispatchEvent(event);
    return event.defaultPrevented;
  }, pasted);
  assert.equal(pastePrevented, false, 'mixed text and file paste must allow native text insertion');
  await expect(page.locator('.composer')).toContainText('pasted.txt');
  await composer.fill(pasted);
  await expect(composer).toHaveValue(pasted);
  await page.getByRole('button', { name: 'Send task message', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().messages.find(message => message.taskId === 'multiline-task')?.text)).toBe(pasted);
  const bubble = page.locator('.message.user .markdown').first();
  await expect(bubble.locator('br')).toHaveCount(1);
  await expect(bubble.locator('p')).toHaveCount(2);
  assert.equal(await bubble.locator('p').first().innerHTML(), 'First line<br>Second line');
  assert.equal(await page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'sendMessage').at(-1)?.args.text), pasted);

  console.log('Multiline composer value, mixed clipboard paste, sent text, and user bubble line breaks passed.');
} finally {
  await browser.close();
}
