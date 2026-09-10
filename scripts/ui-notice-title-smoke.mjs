import {chromium,expect} from '@playwright/test';
import {readFileSync} from 'node:fs';
const browser=await chromium.launch({headless:true});
try{
const page=await browser.newPage({viewport:{width:1440,height:900}});
await page.addInitScript({content:readFileSync('scripts/ui-fixture.js','utf8')+`const q=window.__MONITTER_QA__,s=q.snapshot(),n=Date.now();s.tasks=[{id:'title-test',agentId:'atlas',title:'Title test',nativeSessionId:null,status:'completed',archived:false,createdAt:n,updatedAt:n,parentTaskId:null,channelId:null,projectId:null,hostId:'local',cwd:'/tmp',provider:'codex',model:'',sandbox:'read-only'}];q.setSnapshot(s);`});
await page.goto('http://127.0.0.1:18433');await page.locator('.sidebar .task-select').click({timeout:60000});
const pencil=page.locator('.task-title-edit');await expect(pencil).toHaveCSS('opacity','0');await page.locator('.task-title').hover();await expect(pencil).toHaveCSS('opacity','1');await pencil.click();await expect(page.getByRole('dialog')).toBeVisible();await page.keyboard.press('Escape');
async function name(){await page.getByLabel('Task message',{exact:true}).fill('/autoname');await page.keyboard.press('Enter');}
await name();const notice=page.locator('.alert.notice'),close=notice.getByRole('button',{name:'Dismiss notice'});await expect(notice).toBeVisible();await expect(close).toHaveCSS('opacity','0');
expect((await notice.locator('.icon-slot').boundingBox()).width).toBe((await close.boundingBox()).width);
await notice.hover();await expect(close).toHaveCSS('opacity','1');await page.waitForTimeout(4500);await expect(notice).toBeVisible();await page.mouse.move(1,1);await expect(notice).toHaveCount(0,{timeout:6000});
await page.evaluate(()=>{window.__MONITTER_BRIDGE__.autoname=async()=>{throw new Error('Blocking test error');};});await name();const error=page.locator('.alert.error');await expect(error).toBeVisible();await page.waitForTimeout(4500);await expect(error).toBeVisible();
console.log('Task title pencil, equal notification slots, hover controls, timed notice dismissal and persistent errors passed.');
}finally{await browser.close();}
