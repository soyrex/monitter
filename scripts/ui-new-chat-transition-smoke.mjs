import { chromium, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';

const browser=await chromium.launch({headless:true});
try {
  const page=await browser.newPage({viewport:{width:1200,height:800}});
  const fixture=readFileSync('scripts/ui-fixture.js','utf8');
  await page.addInitScript({content:fixture+`;const original=window.__MONITTER_BRIDGE__.createTask;window.__MONITTER_BRIDGE__.createTask=async input=>{await new Promise(resolve=>window.__releaseTaskCreation=resolve);return original(input);};`});
  await page.goto(process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18450',{waitUntil:'domcontentloaded',timeout:120000});
  await page.getByRole('button',{name:'New chat with Atlas',exact:true}).click({timeout:180000});
  await page.getByLabel('Task message',{exact:true}).fill('Inspect the workspace now');
  await page.getByRole('button',{name:'Send task message',exact:true}).click();

  const starting=page.getByRole('region',{name:'Starting chat',exact:true});
  await expect(starting).toBeVisible();
  await expect(page.getByRole('region',{name:'New chat draft',exact:true})).toHaveCount(0);
  await expect(starting.getByRole('heading',{name:'Inspect the workspace now',exact:true})).toBeVisible();
  await expect(starting.getByText('Inspect the workspace now',{exact:true})).toHaveCount(2);
  await expect(starting.getByText('Sending',{exact:true})).toBeVisible();

  await page.evaluate(()=>window.__releaseTaskCreation());
  await expect(starting).toHaveCount(0);
  await expect(page.locator('.task-layout')).toBeVisible();
  console.log('new-chat send immediately transitions from setup to the chat transcript');
} finally {
  await browser.close();
}
