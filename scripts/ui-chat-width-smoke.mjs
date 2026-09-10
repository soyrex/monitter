import {chromium,expect} from '@playwright/test';
import {readFileSync} from 'node:fs';
const browser=await chromium.launch({headless:true});
try {
 const page=await browser.newPage({viewport:{width:2400,height:1000}});
 await page.addInitScript({content:readFileSync('scripts/ui-fixture.js','utf8')+`const q=window.__MONITTER_QA__,s=q.snapshot(),n=Date.now();s.tasks.push({id:'width',agentId:'atlas',title:'Width test',nativeSessionId:null,status:'idle',archived:false,createdAt:n,updatedAt:n,parentTaskId:null,channelId:null,projectId:null,hostId:'local',cwd:'/tmp',provider:'codex',model:'',sandbox:'read-only'});q.setSnapshot(s);`});
 await page.goto('http://127.0.0.1:18433');await page.locator('.sidebar .task-select').click({timeout:60000});
 const content=page.locator('.message-content'),composer=page.locator('.conversation > .composer');
 await expect.poll(async()=>Math.round((await composer.boundingBox()).width)).toBe(900);
 expect(Math.round((await content.boundingBox()).width)).toBe(900);
 await page.evaluate(()=>document.documentElement.style.setProperty('--chat-font-ratio','1.2'));
 await expect.poll(async()=>Math.round((await composer.boundingBox()).width)).toBe(900);
 expect(Math.round((await content.boundingBox()).width)).toBe(900);
 await page.setViewportSize({width:1000,height:900});
 const pane=await page.locator('.conversation').boundingBox(),input=await composer.boundingBox(),messages=await content.boundingBox();
 expect(Math.round(input.x-pane.x)).toBe(20);expect(Math.round(pane.width-input.width)).toBe(40);
 expect(messages.x).toBeCloseTo(input.x,0);expect(messages.width).toBeCloseTo(input.width,0);
 console.log('Chat and composer share fixed 900px maximum, independent of typography, and retain narrow pane padding.');
} finally {await browser.close();}
