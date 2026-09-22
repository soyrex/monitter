import { chromium, expect } from '@playwright/test';
import { existsSync, readFileSync } from 'node:fs';

const systemChrome = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const browser = await chromium.launch({
  headless: true,
  ...(existsSync(systemChrome) ? { executablePath: systemChrome } : {}),
});
try {
  const page = await browser.newPage({ viewport: { width: 1200, height: 850 } });
  await page.addInitScript({
    content: readFileSync('scripts/ui-fixture.js', 'utf8') + `
      const q = window.__MONITTER_QA__, s = q.snapshot(), n = Date.now();
      s.tasks = [{id:'steer-task',agentId:'atlas',title:'Steering transcript',nativeSessionId:'native',status:'running',archived:false,createdAt:n-1000,updatedAt:n,parentTaskId:null,channelId:null,projectId:null,hostId:'local',cwd:'/tmp',provider:'acp',model:'',sandbox:'read-only'}];
      s.messages = [{id:'request',taskId:'steer-task',role:'user',text:'Original request',createdAt:n-900,attachments:[]}];
      s.queuedMessages = [
        {id:'steer-1',taskId:'steer-task',channelId:null,text:'Take the safer route',attachmentIds:['attachment-1'],createdAt:n-100,status:'sending',error:null,origin:'steering'},
        {id:'queue-1',taskId:'steer-task',channelId:null,text:'Ordinary queued follow-up',attachmentIds:[],createdAt:n-50,status:'queued',error:null}
      ];
      q.setSnapshot(s);
    `,
  });
  await page.goto('http://127.0.0.1:18433');
  await page.locator('.sidebar .task-select').filter({ hasText: 'Steering transcript' }).click({ timeout: 60_000 });

  const feed = page.getByRole('feed', { name: 'Conversation transcript' });
  await expect(feed).toContainText('Take the safer route');
  await expect(feed.getByRole('status', { name: 'Steering' })).toBeVisible();
  await expect(feed).toContainText('1 attachment');
  const queue = page.getByRole('region', { name: 'Queued messages' });
  await expect(queue).toContainText('Ordinary queued follow-up');
  await expect(queue).not.toContainText('Take the safer route');

  await page.evaluate(() => {
    const q = window.__MONITTER_QA__, s = q.snapshot();
    s.queuedMessages.find(message => message.id === 'steer-1').status = 'queued';
    q.setSnapshot(s);
  });
  await expect(feed.getByRole('status', { name: 'Queued' })).toBeVisible();
  await expect(feed.getByText('Take the safer route', { exact: true })).toHaveCount(1);

  await page.evaluate(() => {
    const q = window.__MONITTER_QA__, s = q.snapshot();
    const pending = s.queuedMessages.find(message => message.id === 'steer-1');
    s.queuedMessages = s.queuedMessages.filter(message => message.id !== 'steer-1');
    s.messages.push({id:'sent-1',taskId:'steer-task',role:'user',text:pending.text,createdAt:pending.createdAt,attachments:[]});
    q.setSnapshot(s);
  });
  await expect(feed.getByRole('status', { name: 'Steering' })).toHaveCount(0);
  await expect(feed.getByRole('status', { name: 'Queued' })).toHaveCount(0);
  await expect(feed.getByText('Take the safer route', { exact: true })).toHaveCount(1);

  console.log('Steering follow-up stayed in the transcript, changed state in place, and was not duplicated.');
} finally {
  await browser.close();
}
