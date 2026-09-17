import {chromium,expect} from '@playwright/test';
const browser=await chromium.launch({headless:true});
try {
  const page=await browser.newPage({viewport:{width:1500,height:950}}), errors=[];
  page.on('pageerror',error=>errors.push(error.message));
  await page.addInitScript({path:'scripts/ui-fixture.js'}); await page.addInitScript(()=>{const q=window.__MONITTER_QA__,s=q.snapshot();s.settings.shortcutMode='vim';q.setSnapshot(s);});await page.goto('http://127.0.0.1:18433');
  await expect(page.getByRole('button',{name:'New terminal',exact:true})).toBeVisible({timeout:60000});
  async function command(value) {
    await page.keyboard.press('Escape'); await page.keyboard.type(':');
    const input=page.getByRole('textbox',{name:'Monitter command'}); await expect(input).toBeVisible();
    await input.fill(value); await page.keyboard.press('Enter');
  }
  await command('tabnew');
  const composer=page.locator('.pane-leaf').first().getByRole('textbox',{name:'Task message',exact:true}); await composer.fill('Draft remains here');
  await command('split');
  await expect(page.locator('.pane-leaf')).toHaveCount(2);
  await expect(page.locator('.pane-leaf').last().getByRole('region',{name:'Choose pane content'})).toBeVisible();
  await page.locator('.pane-leaf').first().locator('[data-tab-kind="draft"] .tab').click();
  await expect(composer).toHaveValue('Draft remains here');
  await command('terminal'); await expect(page.locator('.terminal-pane')).toBeVisible({timeout:20000});
  // Real terminal keys remain untouched; close it through its normal tab affordance.
  await page.getByRole('button',{name:/Close terminal/}).click();
  await expect(page.locator('.terminal-pane')).toHaveCount(0);
  await command('q');
  await command('tabnew'); await command('tabnew');
  const before=await page.locator('.tab-entry.active').getAttribute('data-tab-id'); await command('tabprevious');
  expect(await page.locator('.tab-entry.active').getAttribute('data-tab-id')).not.toBe(before);
  await page.keyboard.press('Escape'); await page.keyboard.type(':'); const input=page.getByRole('textbox',{name:'Monitter command'}); await input.fill('wat'); await page.keyboard.press('Enter');
  await expect(page.getByText(/Unknown Monitter command/)).toBeVisible(); await expect(input).toHaveValue('wat');
  expect(errors).toEqual([]); console.log('Vim command mode: draft, split, terminal, quit, tab navigation and invalid command handling passed.');
} finally { await browser.close(); }
