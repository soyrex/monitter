import {chromium,expect} from '@playwright/test';
const browser=await chromium.launch({headless:true});
try{
 const page=await browser.newPage({viewport:{width:1500,height:950}});await page.addInitScript({path:'scripts/ui-fixture.js'});await page.goto('http://127.0.0.1:18433');await page.getByRole('button',{name:'Preferences',exact:true}).waitFor({timeout:60000});
 await page.keyboard.press('Escape');await page.keyboard.type(':');await expect(page.getByRole('textbox',{name:'Monitter command'})).toHaveCount(0);
 await page.getByRole('button',{name:'Preferences',exact:true}).click();await page.getByRole('button',{name:'Permissions & behaviour',exact:true}).click();
 const mode=page.getByRole('combobox',{name:'Shortcut mode'});await expect(mode).toHaveValue('standard');await mode.selectOption('vim');await expect.poll(()=>page.evaluate(()=>window.__MONITTER_QA__.snapshot().settings.shortcutMode)).toBe('vim');
 await page.keyboard.press('Meta+w');await page.waitForTimeout(700);await expect(mode).toBeVisible();await page.keyboard.press('Escape');
 async function command(value){await page.keyboard.press('Escape');await page.keyboard.type(':');const input=page.getByRole('textbox',{name:'Monitter command'});await expect(input).toBeVisible();await input.fill(value);await input.press('Enter');}
 await command('vsplit');await expect(page.locator('.pane-leaf')).toHaveCount(2);
 await page.keyboard.press('Meta+w');await page.waitForTimeout(700);await expect(page.locator('.pane-leaf')).toHaveCount(2);await page.keyboard.press('ArrowLeft');await expect(page.locator('.pane-leaf.active')).toHaveAttribute('data-pane-id','main');
 await page.keyboard.press('Escape');await page.keyboard.type(':');const input=page.getByRole('textbox',{name:'Monitter command'});await expect(page.locator('.vim-commandbar form')).toHaveText(':');await input.fill('tabn');await input.press('Tab');expect(await input.inputValue()).not.toBe('tabn');await expect(input).toBeFocused();await input.press('Escape');
 await mode.selectOption('standard');await expect.poll(()=>page.evaluate(()=>window.__MONITTER_QA__.snapshot().settings.shortcutMode)).toBe('standard');await page.keyboard.press('Meta+w');await expect(mode).toHaveCount(0);await page.keyboard.press('Escape');await page.keyboard.type(':');await expect(input).toHaveCount(0);
 console.log('Shortcut mode setting, non-destructive Vim prefix, pane focus, colon-only command UI, completion and standard close passed.');
}finally{await browser.close();}
