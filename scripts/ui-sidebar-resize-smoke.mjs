import {chromium,expect} from '@playwright/test';
import {readFileSync} from 'node:fs';
const browser=await chromium.launch({headless:true});
try{
 const page=await browser.newPage({viewport:{width:1600,height:1000}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.addInitScript({content:readFileSync('scripts/ui-fixture.js','utf8')+`const q=window.__MONITTER_QA__,s=q.snapshot(),n=Date.now();s.tasks.push({id:'resize',agentId:'atlas',title:'Resize test',nativeSessionId:null,status:'idle',archived:false,createdAt:n,updatedAt:n,parentTaskId:null,channelId:null,projectId:null,hostId:'local',cwd:'/tmp',provider:'codex',model:'',sandbox:'read-only'});q.setSnapshot(s);`});
 await page.goto('http://127.0.0.1:18433',{timeout:60000});await page.locator('.sidebar .task-select').click({timeout:60000});
 const left=page.locator('.sidebar'),right=page.locator('.run-detail');
 async function drag(label,x){const handle=page.getByRole('separator',{name:label,exact:true});const box=await handle.boundingBox();await page.mouse.move(box.x+box.width/2,box.y+box.height/2);await page.mouse.down();await page.mouse.move(x,box.y+box.height/2,{steps:10});await page.mouse.up();}
 await drag('Resize main sidebar',1200);expect((await left.boundingBox()).width).toBeCloseTo(640,0);
 await drag('Resize main sidebar',181);expect((await left.boundingBox()).width).toBeCloseTo(230,0);
 await drag('Resize main sidebar',177);expect((await left.boundingBox()).width).toBeLessThan(180);
 await expect(page.getByRole('separator',{name:'Resize main sidebar',exact:true})).toBeVisible();
 await page.reload();await expect(page.locator('.sidebar-collapsed')).toBeVisible({timeout:60000});
 await drag('Resize main sidebar',200);
 await drag('Resize main sidebar',350);expect(Math.abs((await left.boundingBox()).width-350)).toBeLessThanOrEqual(2);
 const beforeCancel=Math.round((await left.boundingBox()).width);
 const cancelHandle=await page.getByRole('separator',{name:'Resize main sidebar',exact:true}).boundingBox();
 await page.mouse.move(cancelHandle.x+cancelHandle.width/2,cancelHandle.y+100);await page.mouse.down();await page.mouse.move(170,cancelHandle.y+100,{steps:5});
 await page.keyboard.press('Escape');await page.mouse.up();
 await expect.poll(async()=>Math.round((await left.boundingBox()).width)).toBe(beforeCancel);
 if(await page.getByRole('button',{name:'Show right sidebar',exact:true}).count())await page.getByRole('button',{name:'Show right sidebar',exact:true}).click();
 await drag('Resize right sidebar',1400);expect((await right.boundingBox()).width).toBeCloseTo(260,0);
 await drag('Resize right sidebar',800);expect((await right.boundingBox()).width).toBeCloseTo(640,0);
 await drag('Resize right sidebar',1200);expect(Math.abs((await right.boundingBox()).width-400)).toBeLessThanOrEqual(2);
 await page.reload();await expect(left).toBeVisible();await expect(right).toBeVisible();expect(Math.abs((await left.boundingBox()).width-350)).toBeLessThanOrEqual(2);expect(Math.abs((await right.boundingBox()).width-400)).toBeLessThanOrEqual(2);
 await page.keyboard.press('Meta+p');await page.getByRole('button',{name:'Two columns',exact:true}).click();
 const main=page.locator('.pane-leaf[data-pane-id=main]');await main.getByRole('button',{name:'Show right sidebar',exact:true}).click();
 const blade=main.locator('.run-detail');await expect(blade).toBeVisible();expect((await blade.boundingBox()).width).toBeLessThan((await main.boundingBox()).width);
 await main.getByRole('separator',{name:'Resize right sidebar',exact:true}).press('Home');expect((await blade.boundingBox()).width).toBeCloseTo(260,0);
 expect(errors).toEqual([]);console.log('Sidebar pointer resizing, minimums, 40% cap, saved widths and narrow-pane blade passed.');
}finally{await browser.close();}
