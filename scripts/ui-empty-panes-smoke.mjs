import {chromium,expect} from '@playwright/test';
const browser=await chromium.launch({headless:true});
try {
 const page=await browser.newPage({viewport:{width:1600,height:1000}}),errors=[];page.on('pageerror',e=>errors.push(e.message));await page.addInitScript({path:'scripts/ui-fixture.js'});await page.goto('http://127.0.0.1:18433');
 const panes=page.locator('.pane-leaf'),main=page.locator('.pane-leaf[data-pane-id=main]');
 async function layout(name){await page.getByRole('button',{name:'Pane layout',exact:true}).click();await page.getByRole('menuitem',{name,exact:true}).click();}
 async function drag(source,target){const a=await source.boundingBox(),b=await target.boundingBox();await page.mouse.move(a.x+a.width*.45,a.y+a.height/2);await page.mouse.down();await page.mouse.move(b.x+b.width*.5,b.y+b.height*.5,{steps:12});await page.mouse.up();}
 await layout('Two columns');await expect(panes).toHaveCount(2);const second=panes.last();
 await expect(second.getByRole('region',{name:'Choose pane content'})).toBeVisible();await expect(second.getByRole('button',{name:'Overview',exact:true})).toHaveCount(0);
 await expect(main.locator('.dashboard-overview')).toHaveCSS('padding-left','20px');await expect(main.locator('.overview-head')).toHaveCSS('max-width','none');
 await second.getByRole('button',{name:'New chat',exact:true}).click();await second.getByRole('textbox',{name:'Task message',exact:true}).fill('Keep this draft');
 expect(await page.evaluate(()=>window.__MONITTER_QA__.calls.filter(c=>c.method==='createTask'||c.method==='sendMessage').length)).toBe(0);
 await drag(second.locator('.tabs .tab-entry .tab'),main);await expect(panes).toHaveCount(1);await expect(main.getByRole('textbox',{name:'Task message',exact:true})).toHaveValue('Keep this draft');
 await layout('Two columns');await second.getByRole('button',{name:'Terminal',exact:true}).click();await expect(second.locator('.terminal-pane')).toBeVisible();const terminal=await second.locator('.terminal-tab').getAttribute('data-tab-id');
 await drag(second.locator('.terminal-tab .tab'),main);await expect(panes).toHaveCount(1);await expect(main.locator('.terminal-pane')).toBeVisible();expect(await page.evaluate(id=>window.__MONITTER_QA__.calls.filter(c=>c.method==='closeTerminal' && c.id===id).length,terminal)).toBe(0);
 await layout('2 × 2 grid');await expect(panes).toHaveCount(4);await expect(page.getByRole('region',{name:'Choose pane content'})).toHaveCount(3);
 await panes.last().getByRole('button',{name:'Close empty pane',exact:true}).click();await expect(panes).toHaveCount(3);
 await page.reload();await expect(panes).toHaveCount(3);await expect(page.getByRole('region',{name:'Choose pane content'})).toHaveCount(2);expect(errors).toEqual([]);
 console.log('New-pane choices, lazy chat creation, last-tab auto-collapse, terminal preservation, narrow dashboard padding and saved empty panes passed.');
}finally{await browser.close();}
