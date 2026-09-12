import { chromium, expect } from '@playwright/test';
import { readFileSync, mkdirSync } from 'node:fs';
const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
const errors = [];
page.on('pageerror', error => errors.push(error.message));
await page.addInitScript(readFileSync('scripts/ui-fixture.js', 'utf8') + `
  const s = window.__MONITTER_QA__.snapshot();
  s.agents.push({...s.agents[0], id:'south', name:'South'});
  s.tasks = ['Recent chat','Open chat','South chat'].map((title,i)=>({id:'chat-'+i, agentId:i===2?'south':'atlas',title, status:'completed', archived:false, parentTaskId:null,channelId:null,projectId:null,hostId:'local',cwd:'/tmp/monitter-ui-test',provider:'codex',model:'',sandbox:'read-only',nativeSessionId:null,createdAt:1,updatedAt:3-i}));
  window.__MONITTER_QA__.setSnapshot(s);
`);
try {
  await page.goto(process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18435/');
  const group = page.locator('#agent-chats-atlas');
  const open = group.locator('[data-task-id="chat-1"]');
  await expect(open).toHaveClass(/recent/, { timeout: 60000 });
  await open.locator('.task-select').click();
  await expect(open).not.toHaveClass(/recent/);
  await expect(group.locator('.task-row').first()).toHaveAttribute('data-task-id','chat-1');
  await expect(group.locator('.recents-divider')).toHaveText('Recents:');
  await expect(group.locator('.recents-divider svg')).toBeVisible();
  const connector = await group.locator('.recents-divider').evaluate(node => {
    const line = getComputedStyle(node, '::before');
    return { width:line.width, top:line.top, bottom:line.bottom, content:line.content };
  });
  expect(connector).toEqual({width:'1px',top:'-9px',bottom:'-5px',content:'""'});
  const centers = await group.evaluate(node => {
    const clock = node.querySelector('.recents-divider svg').getBoundingClientRect();
    const dot = node.querySelector('.task-select > .dot').getBoundingClientRect();
    return {clock:clock.x + clock.width / 2, dot:dot.x + dot.width / 2};
  });
  expect(Math.abs(centers.clock - centers.dot)).toBeLessThan(0.75);
  await page.mouse.move(900,100);
  await expect(group.locator('[data-task-id="chat-0"]')).toHaveCSS('opacity','0.65');
  await page.locator('[data-task-id="chat-2"] .task-select').click();
  await expect(open).not.toHaveClass(/recent/);
  await page.getByRole('button',{name:'Open agent Atlas',exact:true}).click();
  await page.locator('[data-tab-id="chat-1"]').hover();
  await page.getByRole('button',{name:'Close tab Open chat',exact:true}).click();
  await expect(open).toHaveClass(/recent/);
  await open.locator('.task-select').click();
  mkdirSync('verification',{recursive:true});
  await page.screenshot({path:'verification/sidebar-recents.png'});
  expect(errors).toEqual([]);
  console.log('Sidebar open-first grouping, dimmed recents, hidden workspace tabs and close/reopen checks passed.');
} finally { await browser.close(); }
