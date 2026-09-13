import { chromium, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';

const browser=await chromium.launch({headless:true});
try {
  const page=await browser.newPage({viewport:{width:500,height:780}});
  const fixture=readFileSync('scripts/ui-fixture.js','utf8');
  await page.addInitScript({content:fixture+`;const q=window.__MONITTER_QA__,s=q.snapshot(),n=Date.now();s.tasks=['First','Second'].map((title,index)=>({id:'responsive-'+index,agentId:'atlas',title,nativeSessionId:null,status:'completed',archived:false,createdAt:n+index,updatedAt:n+index,parentTaskId:null,channelId:null,projectId:null,hostId:'local',cwd:'/tmp',provider:'codex',model:'',sandbox:'read-only'}));q.setSnapshot(s);`});
  await page.goto(process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18450',{waitUntil:'domcontentloaded',timeout:120000});
  await page.locator('.sidebar .task-select').filter({hasText:'First'}).click({timeout:60000});
  const workspace=page.locator('.workspace').first();
  await expect(workspace).not.toHaveClass(/compact-tabs/);
  await expect(workspace.locator('.tab-picker-trigger')).toBeHidden();
  await expect(workspace.locator('.tab-entry')).toHaveCount(1);

  await page.getByRole('button',{name:'Back to chats',exact:true}).click();
  await page.locator('.sidebar .task-select').filter({hasText:'Second'}).click();
  await expect(workspace).toHaveClass(/compact-tabs/);
  await expect(workspace.locator('.tab-picker-trigger')).toBeVisible();
  await expect(workspace.locator('.tab-entry')).toHaveCount(2);
  console.log('mobile tab picker appears only for panes with multiple tabs');
} finally {
  await browser.close();
}
