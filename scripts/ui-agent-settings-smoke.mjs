import {chromium,expect} from '@playwright/test';
import {readFileSync} from 'node:fs';
const browser=await chromium.launch({headless:true});
try{
 const page=await browser.newPage({viewport:{width:1440,height:950}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.addInitScript({content:readFileSync('scripts/ui-fixture.js','utf8')+`const q=window.__MONITTER_QA__,s=q.snapshot();s.agents.push({...s.agents[0],id:'beta',name:'Beta',instructions:'Beta instructions'});q.setSnapshot(s);`});
 await page.goto('http://127.0.0.1:18433');await page.getByRole('button',{name:'Edit Atlas',exact:true}).click();
 const settings=page.locator('.settings-pane'),selector=settings.getByRole('combobox',{name:'Select agent',exact:true}),name=settings.getByRole('textbox',{name:'Name',exact:true});
 await expect(settings).toBeVisible();await expect(selector).toHaveValue('atlas');await expect(name).toHaveValue('Atlas');await expect(page.getByRole('dialog',{name:'Edit agent'})).toHaveCount(0);
 await name.fill('Atlas draft');await selector.selectOption('beta');await expect(name).toHaveValue('Beta');await selector.selectOption('atlas');await expect(name).toHaveValue('Atlas draft');
 await settings.getByRole('button',{name:'Save agent',exact:true}).click();await expect(page.getByRole('button',{name:'Edit Atlas draft',exact:true})).toBeVisible();
 await name.fill('Keep edits');await page.keyboard.press('Meta+,');await expect(name).toHaveValue('Keep edits');
 await settings.getByRole('button',{name:'Appearance',exact:true}).click();await settings.getByRole('button',{name:'Agents',exact:true}).click();await expect(name).toHaveValue('Keep edits');
 const tab=page.locator('.settings-tab .tab'),main=page.locator('.pane-leaf[data-pane-id=main]');const a=await tab.boundingBox(),b=await main.boundingBox();await page.mouse.move(a.x+a.width/2,a.y+a.height/2);await page.mouse.down();await page.mouse.move(b.x+b.width*.98,b.y+b.height/2,{steps:10});await page.mouse.up();await expect(page.locator('.pane-leaf')).toHaveCount(2);await expect(name).toHaveValue('Keep edits');
 await page.reload();await expect(settings).toBeVisible();await expect(name).toHaveValue('Keep edits');
 await settings.getByRole('button',{name:'Discard changes',exact:true}).click();await expect(name).toHaveValue('Atlas');
 await settings.getByRole('button',{name:'New agent',exact:true}).click();await name.fill('New colleague');await settings.getByRole('button',{name:'Save agent',exact:true}).click();await expect(page.getByRole('button',{name:'Edit New colleague',exact:true})).toBeVisible();await expect(selector).not.toHaveValue('');
 expect(errors).toEqual([]);console.log('Agent editor in Settings: selection, independent drafts, save/create, category navigation, pane move, reload and discard passed.');
}finally{await browser.close();}
