import {chromium,expect} from '@playwright/test';
import {readFileSync} from 'node:fs';
const browser=await chromium.launch({headless:true});
try {
 const page=await browser.newPage({viewport:{width:1440,height:900}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.addInitScript({content:readFileSync('scripts/ui-fixture.js','utf8')+`
 const q=window.__MONITTER_QA__,s=q.snapshot();s.channels=[{id:'channel-persist',name:'Team channel',description:'',agentIds:['atlas'],messages:[]}];q.setSnapshot(s);
 window.__workspaceWrites=0;const originalSet=Storage.prototype.setItem;Storage.prototype.setItem=function(key,value){if(key==='monitter.workspace.v1')window.__workspaceWrites++;return originalSet.call(this,key,value);};`});
 await page.goto('http://127.0.0.1:18433');
 await page.locator('.sidebar').getByRole('button',{name:/Team channel/}).click();
 const input=page.getByRole('textbox',{name:'Channel message',exact:true});await input.fill('Unsent channel draft');
 await page.locator('.recipient-picker').getByRole('button',{name:'Atlas',exact:true}).click();
 await expect(page.getByRole('button',{name:'Send channel message',exact:true})).toBeEnabled();
 await page.getByRole('button',{name:'Monitter menu',exact:true}).click();await expect(page.getByRole('menuitem',{name:'Preferences',exact:true})).toBeVisible();
 await page.keyboard.press('Escape');await page.waitForTimeout(200);const writes=await page.evaluate(()=>window.__workspaceWrites);await page.waitForTimeout(350);expect(await page.evaluate(()=>window.__workspaceWrites)-writes).toBeLessThan(3);
 await page.reload();await expect(input).toHaveValue('Unsent channel draft');await expect(page.locator('.recipient-picker').getByRole('button',{name:'Atlas',exact:true})).toHaveAttribute('aria-pressed','true');
 await page.keyboard.press('Meta+,');await expect(page.locator('.settings-pane')).toBeVisible();await page.getByRole('button',{name:'Close Settings tab',exact:true}).click();await expect(input).toHaveValue('Unsent channel draft');
 await page.getByRole('button',{name:'Monitter menu',exact:true}).click();await expect(page.getByRole('menuitem',{name:'Preferences',exact:true})).toBeVisible();expect(errors).toEqual([]);
 console.log('Channel recipient/draft persistence is bounded; channel reload and clicks remain responsive, including Settings and menus.');
}finally{await browser.close();}
