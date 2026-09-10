import { chromium, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';
const browser=await chromium.launch({headless:true}), page=await browser.newPage({viewport:{width:1440,height:900}}), errors=[];
page.on('pageerror',error=>errors.push(error.message));
async function gesture(source,target,position={x:.25,y:.5},release=true) {
 const a=await source.boundingBox(),b=await target.boundingBox();
 await page.mouse.move(a.x+a.width*.45,a.y+a.height*.5);await page.mouse.down();
 await page.mouse.move(b.x+b.width*position.x,b.y+b.height*position.y,{steps:12});
 if(release)await page.mouse.up();
}
const entries=pane=>pane.locator('.tabs .tab-entry:not(.dashboard-tab)');
const ids=pane=>entries(pane).evaluateAll(nodes=>nodes.map(n=>n.dataset.tabId));
const tab=(pane,id)=>pane.locator(`.tabs [data-tab-id="${id}"] .tab`);
try {
 await page.addInitScript({content:readFileSync('scripts/ui-fixture.js','utf8')+`const q=window.__MONITTER_QA__,s=q.snapshot(),n=Date.now();for(const id of ['tab-a','tab-b'])s.tasks.push({id,agentId:'atlas',title:id,nativeSessionId:null,status:'idle',archived:false,createdAt:n,updatedAt:n,parentTaskId:null,channelId:null,projectId:null,hostId:'local',cwd:'/tmp',provider:'codex',model:'',sandbox:'read-only'});q.setSnapshot(s);window.nativeDrags=0;window.addEventListener('dragstart',()=>window.nativeDrags++);`});
 await page.goto(process.env.MONITTER_TEST_URL||'http://127.0.0.1:18433');
 for(const name of ['tab-a','tab-b'])await page.locator('.sidebar .task-select').filter({hasText:name}).click();
 const main=page.locator('.pane-leaf[data-pane-id=main]');
 const dashboard=main.getByRole('button',{name:'Overview',exact:true});await expect(dashboard).toHaveText('');await expect(dashboard.locator('svg')).toHaveCount(1);
 await main.getByRole('textbox',{name:'Task message',exact:true}).fill('Draft survives a move');
 await page.keyboard.press('Meta+,');await expect(entries(main)).toHaveCount(3);
 await gesture(tab(main,'tab-b'),tab(main,'tab-a'),{x:.2,y:.5},false);await expect(page.locator('[data-tab-insert]')).toHaveCount(1);await page.mouse.up();expect(await ids(main)).toEqual(['tab-b','tab-a','settings']);
 await tab(main,'tab-b').click();expect(await ids(main)).toEqual(['tab-b','tab-a','settings']);await expect(page.locator('.pane-leaf')).toHaveCount(1);
 await expect(main.getByRole('textbox',{name:'Task message',exact:true})).toHaveValue('Draft survives a move');
 await gesture(tab(main,'settings'),main,{x:.98,y:.5},false);await expect(main.locator('.pane-drop')).toHaveAttribute('data-edge','right');await page.keyboard.press('Escape');await page.mouse.up();await expect(page.locator('.pane-leaf')).toHaveCount(1);await expect(page.locator('.pane-drop')).toHaveCount(0);
 await gesture(tab(main,'settings'),main,{x:.98,y:.5});await expect(page.locator('.pane-leaf')).toHaveCount(2);
 const second=page.locator('.pane-leaf').last();
 await gesture(tab(main,'tab-b'),tab(second,'settings'),{x:.2,y:.5});await expect(tab(second,'tab-b')).toBeVisible();expect(await ids(second)).toEqual(['tab-b','settings']);await expect(second.getByRole('textbox',{name:'Task message',exact:true})).toHaveValue('Draft survives a move');
 await gesture(tab(second,'tab-b'),tab(second,'settings'),{x:.9,y:.5});expect(await ids(second)).toEqual(['settings','tab-b']);
 await tab(second,'settings').click();expect(await ids(second)).toEqual(['settings','tab-b']);
 await gesture(tab(main,'tab-a'),second,{x:.5,y:.5});await expect(tab(second,'tab-a')).toBeVisible();expect(await ids(second)).toEqual(['settings','tab-b','tab-a']);
 await page.reload();await expect(page.locator('.pane-leaf')).toHaveCount(2);await expect(entries(second)).toHaveCount(3);expect(await ids(second)).toEqual(['settings','tab-b','tab-a']);
 await tab(second,'tab-b').click();await expect(second.getByRole('textbox',{name:'Task message',exact:true})).toHaveValue('Draft survives a move');
 // Match sidebar toggle dimensions even under native Mac scaling overrides.
 await page.locator('main.app-shell').first().evaluate(el=>el.classList.add('native-mac'));
 const left=page.getByRole('button',{name:'Collapse main sidebar',exact:true}),right=main.getByRole('button',{name:/^(Show|Hide) right sidebar$/});
 expect((await left.boundingBox()).width).toBeCloseTo((await right.boundingBox()).width,1);expect((await left.locator('svg').boundingBox()).width).toBeCloseTo((await right.locator('svg').boundingBox()).width,1);
 expect(await page.evaluate(()=>window.nativeDrags)).toBe(0);expect(errors).toEqual([]);
 console.log('Real pointer tab reorder, embedded pane reorder, edge split, cancellation, cross-pane insertion, draft persistence, stable activation, icon-only dashboard and matching sidebar controls passed.');
} finally {await browser.close();}
