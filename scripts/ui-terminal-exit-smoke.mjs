import {chromium,expect} from '@playwright/test';
const browser=await chromium.launch({headless:true});
try {
 const page=await browser.newPage({viewport:{width:1440,height:900}});await page.addInitScript({path:'scripts/ui-fixture.js'});await page.goto('http://127.0.0.1:18433');await expect(page.getByRole('button',{name:'Monitter menu',exact:true})).toBeVisible({timeout:60000});
 await page.getByRole('button',{name:'New terminal',exact:true}).click();await expect(page.locator('.terminal-pane .xterm-helper-textarea')).toBeAttached({timeout:20000});await page.locator('.terminal-pane .xterm-helper-textarea').focus();await page.keyboard.press('Control+d');
 await expect(page.locator('.terminal-tab')).toHaveCount(0,{timeout:15000});await expect(page.locator('.terminal-pane')).toHaveCount(0);
 expect(await page.evaluate(()=>window.__MONITTER_QA__.calls.filter(c=>c.method==='closeTerminal').length)).toBe(1);
 expect(await page.evaluate(()=>window.__MONITTER_QA__.terminals())).toHaveLength(0);
 console.log('Ctrl-D shell exit automatically closes the tab and releases its terminal exactly once.');
}finally{await browser.close();}
