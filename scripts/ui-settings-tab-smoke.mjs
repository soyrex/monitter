import {chromium,webkit,expect} from '@playwright/test';
import {readFileSync,writeFileSync} from 'node:fs';
const browser=await (process.env.MONITTER_TEST_BROWSER === 'webkit' ? webkit : chromium).launch({headless:true}), results=[],errors=[];
const page=await browser.newPage({viewport:{width:1440,height:900}});
async function chooseCategory(label,id) {
 const trigger=page.getByRole('button',{name:'Preferences category',exact:true});
 if(await trigger.isVisible()){await trigger.click();await page.locator('.phone-category-list').getByRole('button',{name:label,exact:true}).click();}
 else await page.getByRole('navigation',{name:'Settings categories'}).getByRole('button',{name:label,exact:true}).click();
}

page.on('pageerror',error=>errors.push(error.message));
try {
 await page.addInitScript(()=>{window.__TAURI_INTERNALS__={transformCallback:()=>1,invoke:async()=>1};});
 await page.addInitScript({content:readFileSync('scripts/ui-fixture.js','utf8')+`
 const q=window.__MONITTER_QA__,s=q.snapshot();s.tasks.push({id:'settings-chat',agentId:'atlas',title:'Keep working',nativeSessionId:null,status:'idle',archived:false,createdAt:Date.now(),updatedAt:Date.now(),parentTaskId:null,channelId:null,projectId:null,hostId:'local',cwd:'/tmp',provider:'codex',model:'',sandbox:'read-only'});q.setSnapshot(s);`});
 await page.goto(process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18433',{timeout:60000});
 await page.locator('.sidebar [data-task-id="settings-chat"] .task-select').click();
 await page.getByLabel('Task message',{exact:true}).fill('Keep this draft');
 await page.keyboard.press('Meta+,');
 await expect(page.locator('.settings-tab')).toHaveCount(1);await expect(page.getByRole('dialog')).toHaveCount(0);
 const categories=page.getByRole('navigation',{name:'Settings categories'});
 await expect(categories).toBeVisible();await page.keyboard.press('Meta+,');await expect(page.locator('.settings-tab')).toHaveCount(1);
 await page.locator('.settings-pane').getByRole('button',{name:'dark',exact:true}).click();
 await expect.poll(()=>page.evaluate(()=>window.__MONITTER_QA__.snapshot().settings.theme)).toBe('dark');
 await chooseCategory('Fonts & text','typography');
 await page.getByLabel('Chat font',{exact:true}).fill('Georgia');await page.getByLabel('Chat font',{exact:true}).press('Tab');
 await expect.poll(()=>page.evaluate(()=>window.__MONITTER_QA__.snapshot().settings.chatFont)).toBe('Georgia');
 await page.locator('.tabs').getByRole('button',{name:'Keep working',exact:true}).click();
 await expect(page.getByLabel('Task message',{exact:true})).toHaveValue('Keep this draft');await expect(categories).toBeHidden();
 await page.locator('.settings-tab .tab').click();await expect(categories.getByRole('button',{name:/Fonts & text/})).toHaveAttribute('aria-current','page');
 results.push('Settings is a singleton tab; auto-save works and switching preserves chat drafts and category');
 await chooseCategory('Appearance','appearance');
 await page.evaluate(()=>{const bridge=window.__MONITTER_BRIDGE__,original=bridge.saveSettings;bridge.saveSettings=async()=>{bridge.saveSettings=original;throw Error('Save failure fixture');};});
 await page.locator('.settings-pane').getByRole('button',{name:'light',exact:true}).click();
 await expect(page.locator('.settings-pane').getByRole('alert')).toContainText('Save failure fixture');
 await page.locator('.settings-pane').getByRole('button',{name:'light',exact:true}).click();
 await expect.poll(()=>page.evaluate(()=>window.__MONITTER_QA__.snapshot().settings.theme)).toBe('light');
 await expect(page.locator('.settings-pane').getByRole('alert')).toHaveCount(0);
 await chooseCategory('Fonts & text','typography');
 results.push('save errors remain visible and the next save can recover');

 const dt=await page.evaluateHandle(()=>new DataTransfer());await page.locator('.settings-tab .tab').dispatchEvent('dragstart',{dataTransfer:dt});
 const main=page.locator('.pane-leaf[data-pane-id="main"]');const box=await main.boundingBox();const coords={dataTransfer:dt,clientX:box.x+box.width*.98,clientY:box.y+box.height*.5};
 await main.dispatchEvent('dragover',coords);await main.dispatchEvent('drop',coords);
 await expect(page.locator('.pane-leaf')).toHaveCount(2);await expect(page.locator('.pane-leaf').last().locator('.settings-tab')).toHaveCount(1);
 await expect(main.getByRole('button',{name:'Keep working',exact:true})).toBeVisible();
 await page.keyboard.press('Meta+,');await expect(page.locator('.settings-tab')).toHaveCount(1);
 await page.reload();await expect(page.locator('.pane-leaf')).toHaveCount(2);await expect(page.locator('.pane-leaf').last().locator('.settings-tab')).toHaveCount(1);
 await expect(page.locator('.settings-header h1')).toHaveText('Fonts & text');
 results.push('Settings drags to a new split and restores its pane/category after reload');
 await chooseCategory('Agents','agents');
 await expect(page.locator('.pane-leaf').last().getByRole('textbox',{name:'Find agents'})).toBeVisible();await expect(page.locator('.settings-tab')).toHaveCount(1);await expect(page.getByRole('dialog')).toHaveCount(0);
 results.push('Agents category opens the editor in the existing Settings tab in another pane');
 const bounds=await page.locator('.settings-pane').evaluate(el=>({width:el.clientWidth,scroll:el.scrollWidth,bodyOverflow:document.documentElement.scrollHeight>innerHeight}));
 expect(bounds.scroll).toBeLessThanOrEqual(bounds.width+1);expect(bounds.bodyOverflow).toBe(false);
 await chooseCategory('Conversation','conversation');await expect(page.getByRole('switch',{name:'Default to steering busy agents'})).toBeVisible();
 await chooseCategory('Conversation','conversation');await expect(page.getByRole('switch',{name:'Show tool activity'})).toBeVisible();
 await page.screenshot({path:'verification/settings-tab-narrow.png'});
 await page.locator('.settings-tab .tab').click();await page.keyboard.press('Meta+w');await expect(page.locator('.settings-tab')).toHaveCount(0);await expect(page.getByLabel('Task message',{exact:true})).toHaveValue('Keep this draft');
 results.push('Narrow settings pane contains overflow, preserves controls, and Cmd-W closes only Settings');
 await page.keyboard.press('Meta+,');await chooseCategory('Appearance','appearance');
 await page.keyboard.press('Meta+p');await page.getByRole('button',{name:'One pane',exact:true}).click();
 await page.screenshot({path:'verification/settings-tab-wide.png'});
 expect(errors).toEqual([]);console.log(JSON.stringify({results,errors},null,2));writeFileSync('verification/ui-settings-tab.json',JSON.stringify({results,errors},null,2));
}catch(error){await page.screenshot({path:'verification/ui-settings-tab-failure.png'});console.error(errors);throw error;}finally{await browser.close();}
