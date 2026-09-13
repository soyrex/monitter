import { chromium, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';

const browser=await chromium.launch({headless:true});
try {
  const page=await browser.newPage({viewport:{width:1100,height:760}});
  await page.addInitScript({content:readFileSync('scripts/ui-fixture.js','utf8')+`;const q=window.__MONITTER_QA__,s=q.snapshot();s.settings.accent='#ffffff';q.setSnapshot(s);`});
  await page.goto(process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18450',{waitUntil:'domcontentloaded',timeout:120000});
  await page.getByRole('button',{name:'New chat with Atlas',exact:true}).waitFor({timeout:180000});
  const foreground=()=>page.evaluate(()=>getComputedStyle(document.documentElement).getPropertyValue('--on-accent').trim());
  await expect.poll(foreground).toBe('#000');
  await expect.poll(()=>page.locator('.sidebar .avatar').first().evaluate(node=>getComputedStyle(node).color)).toBe('rgb(0, 0, 0)');
  await page.evaluate(()=>{const q=window.__MONITTER_QA__,s=q.snapshot();s.settings.accent='#000000';q.setSnapshot(s);});
  await expect.poll(foreground).toBe('#fff');
  await expect.poll(()=>page.locator('.sidebar .avatar').first().evaluate(node=>getComputedStyle(node).color)).toBe('rgb(255, 255, 255)');
  console.log('accent-filled icons switch between black and white foregrounds');
} finally {
  await browser.close();
}
